use super::base_balancer::*;
use async_channel::{Receiver, Sender};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Mutex, MutexGuard};

// Throttled (limit LIMIT): a site runs at most LIMIT requests; the others wait here, at the main
// server, in arrival order. A freed slot goes straight to the oldest waiter, so a newcomer can
// never take it first. Free slots are tokens (site indices) in a queue: each site starts with
// LIMIT tokens, interleaved (0..n, 0..n, ...), and a returned token goes to the back, so ties
// rotate over the sites. Every operation is O(1)
pub struct Throttled<const LIMIT: usize> {
    state: Mutex<State>,
}

struct State {
    free: VecDeque<usize>, // one token per free slot, next to hand out first
    waiters: VecDeque<Sender<usize>>, // one single-use channel per waiting request, oldest first
}

impl<const LIMIT: usize> Throttled<LIMIT> {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves free / waiters consistent, so keep using them
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // A free slot now (only if nobody is waiting, or it would jump the queue), else a place
    // at the back of the queue. Decided at call time, so the order is the order of the calls
    fn take_or_wait(&self) -> Result<usize, Receiver<usize>> {
        let mut state = self.lock();
        if state.waiters.is_empty()
            && let Some(idx) = state.free.pop_front()
        {
            return Ok(idx);
        }
        let (tx, rx) = async_channel::bounded(1);
        state.waiters.push_back(tx);
        Err(rx)
    }
}

impl<const LIMIT: usize> LoadBalancerPolicy for Throttled<LIMIT> {
    type Ticket = ();
    fn new(offloads: &[OffloadSite]) -> Self {
        const { assert!(LIMIT > 0, "throttled: LIMIT must be at least 1") };
        Self {
            state: Mutex::new(State {
                free: (0..LIMIT).flat_map(|_| 0..offloads.len()).collect(),
                waiters: VecDeque::new(),
            }),
        }
    }
    // sync path (tests, LoadBalancer::select) can't wait: only valid while a slot is free
    fn select(&self, _key: Option<&str>, _workers: &[OffloadSite]) -> (usize, ()) {
        match self.take_or_wait() {
            Ok(idx) => (idx, ()),
            Err(_) => panic!("throttled: no free site, use select_wait"),
        }
    }
    fn select_wait(
        &self,
        _key: Option<&str>,
        _workers: &[OffloadSite],
    ) -> impl Future<Output = (usize, ())> + Send {
        let slot = self.take_or_wait();
        async move {
            let idx = match slot {
                Ok(idx) => idx,
                // the sender is only dropped after it has sent (complete) or with the policy
                Err(rx) => rx.recv().await.expect("throttled: policy dropped"),
            };
            (idx, ())
        }
    }
    fn complete(&self, worker_idx: usize, _ticket: (), _feedback: Option<&TimesFeedback>) {
        let mut state = self.lock();
        // hand the slot to the oldest waiter still there; a closed channel = that request
        // was dropped while waiting, try the next one
        // (not covered: a request dropped after the hand-off but before it read the index)
        while let Some(tx) = state.waiters.pop_front() {
            if tx.try_send(worker_idx).is_ok() {
                return;
            }
        }
        state.free.push_back(worker_idx);
    }
}
pub type ThrottledLB = LoadBalancer<Throttled<1>>;
pub type Throttled2LB = LoadBalancer<Throttled<2>>;

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, Decode, Encode, Render};

    fn sites(n: usize) -> Vec<OffloadSite> {
        let cfg = Config::new(Decode::Gpu, Render::Gpu, Encode::Cpu);
        (0..n)
            .map(|i| OffloadSite::new(format!("site{i}:7010"), format!("s{i}"), cfg))
            .collect()
    }

    #[test]
    fn test_throttled_free_sites_rotate() {
        let s = sites(3);
        let p = Throttled::<1>::new(&s);
        assert_eq!(p.select(None, &s).0, 0);
        assert_eq!(p.select(None, &s).0, 1);
        p.complete(0, (), None); // back of the free queue, behind site 2
        assert_eq!(p.select(None, &s).0, 2);
        assert_eq!(p.select(None, &s).0, 0);
    }

    #[test]
    fn test_throttled2_two_per_site_interleaved() {
        let s = sites(2);
        let p = Throttled::<2>::new(&s);
        let picks: Vec<usize> = (0..4).map(|_| p.select(None, &s).0).collect();
        assert_eq!(picks, [0, 1, 0, 1]);
    }

    #[test]
    #[should_panic(expected = "no free site")]
    fn test_throttled2_sync_select_panics_after_limit() {
        let s = sites(1);
        let p = Throttled::<2>::new(&s);
        p.select(None, &s);
        p.select(None, &s);
        p.select(None, &s);
    }

    #[test]
    fn test_throttled2_waiters_in_arrival_order() {
        let s = sites(2);
        let p = Throttled::<2>::new(&s);
        smol::block_on(async {
            for _ in 0..4 {
                p.select_wait(None, &s).await;
            }
            // all 4 slots busy: three requests wait, in this order
            let w1 = p.select_wait(None, &s);
            let w2 = p.select_wait(None, &s);
            let w3 = p.select_wait(None, &s);
            p.complete(1, (), None);
            p.complete(1, (), None);
            p.complete(0, (), None);
            assert_eq!(w1.await.0, 1);
            assert_eq!(w2.await.0, 1);
            assert_eq!(w3.await.0, 0);
            // nothing was marked free: a fifth request has to wait for the next complete
            let w4 = p.select_wait(None, &s);
            p.complete(0, (), None);
            assert_eq!(w4.await.0, 0);
        });
    }

    #[test]
    #[should_panic(expected = "no free site")]
    fn test_throttled_sync_select_panics_when_full() {
        let s = sites(1);
        let p = Throttled::<1>::new(&s);
        p.select(None, &s);
        p.select(None, &s);
    }

    #[test]
    fn test_throttled_waiters_get_sites_in_arrival_order() {
        let s = sites(2);
        let p = Throttled::<1>::new(&s);
        smol::block_on(async {
            let a = p.select_wait(None, &s).await.0;
            let b = p.select_wait(None, &s).await.0;
            assert_eq!((a, b), (0, 1));
            // both busy: three requests wait, in this order
            let w1 = p.select_wait(None, &s);
            let w2 = p.select_wait(None, &s);
            let w3 = p.select_wait(None, &s);
            p.complete(1, (), None);
            p.complete(0, (), None);
            p.complete(1, (), None);
            // oldest waiter got the first site freed
            assert_eq!(w1.await.0, 1);
            assert_eq!(w2.await.0, 0);
            assert_eq!(w3.await.0, 1);
        });
    }

    #[test]
    fn test_throttled_newcomer_does_not_jump_queue() {
        let s = sites(1);
        let p = Throttled::<1>::new(&s);
        smol::block_on(async {
            p.select_wait(None, &s).await;
            let waiting = p.select_wait(None, &s);
            p.complete(0, (), None); // handed to `waiting`, not marked free
            let newcomer = p.select_wait(None, &s);
            assert_eq!(waiting.await.0, 0);
            p.complete(0, (), None);
            assert_eq!(newcomer.await.0, 0);
        });
    }

    #[test]
    fn test_throttled_skips_dropped_waiter() {
        let s = sites(1);
        let p = Throttled::<1>::new(&s);
        smol::block_on(async {
            p.select_wait(None, &s).await;
            drop(p.select_wait(None, &s)); // gave up while waiting
            let next = p.select_wait(None, &s);
            p.complete(0, (), None);
            assert_eq!(next.await.0, 0);
        });
    }

    #[test]
    fn test_throttled_guard_drop_frees_site() {
        let lb = ThrottledLB::new(sites(1));
        smol::block_on(async {
            {
                let g = lb.select_wait(None).await;
                assert_eq!(g.name, "s0");
            } // dropped without finish -> complete
            assert_eq!(lb.select_wait(None).await.name, "s0");
        });
    }
}

use super::base_balancer::*;
use std::sync::{Mutex, MutexGuard};

// Equally Spread Current Execution: send to the site with the fewest requests in flight
// (least connections). Linear scan, like nginx least_conn; ties go to the lowest index
pub struct ESCE {
    job_vec: Mutex<Vec<usize>>,
}

impl ESCE {
    fn lock(&self) -> MutexGuard<'_, Vec<usize>> {
        // A panic while holding the lock leaves the counts themselves consistent, so keep using them
        self.job_vec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl LoadBalancerPolicy for ESCE {
    type Ticket = ();
    fn new(offloads: &[OffloadSite]) -> Self {
        Self {
            job_vec: Mutex::new(vec![0; offloads.len()]),
        }
    }
    fn select(&self, _key: Option<&str>, _workers: &[OffloadSite]) -> (usize, ()) {
        let mut vec = self.lock();
        let index = (0..vec.len()).min_by_key(|&i| vec[i]).unwrap();
        vec[index] += 1;
        (index, ())
    }
    fn complete(&self, worker_idx: usize, _ticket: (), _feedback: Option<&TimesFeedback>) {
        let mut vec = self.lock();
        // saturating: an unmatched complete must not wrap the count to usize::MAX
        vec[worker_idx] = vec[worker_idx].saturating_sub(1);
    }
}
pub type EsceLB = LoadBalancer<ESCE>;

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
    fn test_esce_least_in_flight() {
        let s = sites(3);
        let p = ESCE::new(&s);
        assert_eq!(p.select(None, &s).0, 0);
        assert_eq!(p.select(None, &s).0, 1);
        p.complete(0, (), None);
        // site 0 is free again, site 1 has a job, site 2 is free: lowest free index wins
        assert_eq!(p.select(None, &s).0, 0);
        assert_eq!(p.select(None, &s).0, 2);
        assert_eq!(p.select(None, &s).0, 0); // all at 1 -> lowest index
    }

    #[test]
    fn test_esce_complete_never_underflows() {
        let s = sites(2);
        let p = ESCE::new(&s);
        p.complete(1, (), None); // unmatched complete
        assert_eq!(p.select(None, &s).0, 0);
        assert_eq!(p.select(None, &s).0, 1);
    }

    #[test]
    fn test_esce_guard_drop_completes() {
        let lb = EsceLB::new(sites(2));
        {
            let g = lb.select(None);
            assert_eq!(g.name, "s0");
        } // dropped without finish -> completes
        assert_eq!(lb.select(None).name, "s0");
    }
}

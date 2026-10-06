// use std::net::ToSocketAddrs;
use std::sync::Arc;

pub trait LoadBalancerPolicy: Send + Sync + Default {
    fn select(&self, key: Option<&str>, num_workers: usize) -> usize;
    fn complete(&self, _worker_idx: usize, _feedback: Option<&TimesFeedback>) {}
}

pub struct TimesFeedback {
    // all used i64 for ms
    pub t_net: i64,
    pub t_queue: i64,
    pub t_work: i64,
    pub t_asset: i64,
}

pub struct LoadBalancer<P: LoadBalancerPolicy> {
    workers: Vec<String>, //vec of endpoint
    policy: Arc<P>,
}

pub struct SelectionGuard<'a, P: LoadBalancerPolicy> {
    pub endpoint: &'a str,
    worker_idx: usize,
    policy: Arc<P>,
    completed: bool,
}

impl<P: LoadBalancerPolicy> LoadBalancer<P> {
    pub fn new<'a>(addrs: impl IntoIterator<Item = &'a str>) -> Self {
        let policy = P::default();
        let addrs: Vec<String> = addrs.into_iter().map(String::from).collect();

        assert!(!addrs.is_empty(), "addrs must not empty");
        // for addr in &addrs {
        //     assert!(addr.to_socket_addrs().is_ok(), "addr must be valid address");
        // }

        Self {
            workers: addrs,
            policy: Arc::new(policy),
        }
    }
    pub fn select(&self, key: Option<&str>) -> SelectionGuard<'_, P> {
        let idx = self.policy.select(key, self.workers.len());
        SelectionGuard {
            endpoint: &self.workers[idx],
            worker_idx: idx,
            policy: Arc::clone(&self.policy),
            completed: false,
        }
    }
}

impl<P: LoadBalancerPolicy> SelectionGuard<'_, P> {
    pub fn finish(mut self, feedback: &TimesFeedback) {
        self.policy.complete(self.worker_idx, Some(feedback));
        self.completed = true;
    }
}

impl<P: LoadBalancerPolicy> Drop for SelectionGuard<'_, P> {
    fn drop(&mut self) {
        if !self.completed {
            self.policy.complete(self.worker_idx, None);
        }
    }
}

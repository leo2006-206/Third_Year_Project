use std::sync::atomic::{AtomicUsize, Ordering};

use super::base_balancer::{LoadBalancer, LoadBalancerPolicy};

#[derive(Default)]
pub struct RoundRobin {
    counter: AtomicUsize,
}

impl LoadBalancerPolicy for RoundRobin {
    fn select(&self, _key: Option<&str>, num_workers: usize) -> usize {
        self.counter.fetch_add(1, Ordering::Relaxed) % num_workers
    }
}
pub type RoundRobinLB = LoadBalancer<RoundRobin>;

use std::sync::atomic::{AtomicUsize, Ordering};

use super::base_balancer::*;

pub struct RoundRobin {
    counter: AtomicUsize,
}

impl LoadBalancerPolicy for RoundRobin {
    fn new(_offloads: &[OffloadSite]) -> Self {
        Self {
            counter: AtomicUsize::default(),
        }
    }
    fn select(&self, _key: Option<&str>, workers: &[OffloadSite]) -> usize {
        self.counter.fetch_add(1, Ordering::Relaxed) % workers.len()
    }
}
pub type RoundRobinLB = LoadBalancer<RoundRobin>;

use std::hash::{DefaultHasher, Hash, Hasher};

use super::base_balancer::{LoadBalancer, LoadBalancerPolicy};

#[derive(Default)]
pub struct KeyHash {}

impl LoadBalancerPolicy for KeyHash {
    fn select(&self, key: Option<&str>, num_workers: usize) -> usize {
        let key = key.expect("key option must contain a value");

        // deterministic hash, fixed seed
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % num_workers
    }
}

pub type KeyHashLB = LoadBalancer<KeyHash>;

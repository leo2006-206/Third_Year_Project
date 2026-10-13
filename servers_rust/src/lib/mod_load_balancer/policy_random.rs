use fastrand::Rng;
use std::sync::Mutex;

use super::base_balancer::{LoadBalancer, LoadBalancerPolicy};

pub struct RandomNumber {
    rng: Mutex<Rng>,
}

impl LoadBalancerPolicy for RandomNumber {
    fn new<S: AsRef<str>>(_endpoints: &[S]) -> Self {
        Self {
            rng: Mutex::new(Rng::with_seed(0)),
        }
    }
    fn select(&self, _key: Option<&str>, num_workers: usize) -> usize {
        let mut rng = self.rng.lock().unwrap();
        rng.usize(0..num_workers)
    }
}

pub type RandomLB = LoadBalancer<RandomNumber>;

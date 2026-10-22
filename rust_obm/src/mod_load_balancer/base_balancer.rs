// use std::net::ToSocketAddrs;
use std::sync::Arc;

pub trait LoadBalancerPolicy: Send + Sync {
    fn new(offloads: &[OffloadSite]) -> Self;
    fn select(&self, key: Option<&str>, num_workers: usize) -> usize;
    fn complete(&self, _worker_idx: usize, _feedback: Option<&TimesFeedback>) {}
}

pub mod config {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Decode {
        Cpu,
        Gpu,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Render {
        Cpu,
        Llvmpipe,
        Gpu,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Encode {
        Cpu,
        Gpu,
    }
    #[derive(Debug, Clone, Copy)]
    pub struct Config {
        pub decode: Decode,
        pub render: Render,
        pub encode: Encode,
    }
    impl Config {
        pub const fn new(decode: Decode, render: Render, encode: Encode) -> Self {
            Self {
                decode,
                render,
                encode,
            }
        }
    }
}
#[derive(Debug, Clone)]
pub struct OffloadSite {
    pub endpoint: String,
    pub config: config::Config,
}
impl OffloadSite {
    pub fn new(endpoint: impl Into<String>, config: config::Config) -> Self {
        Self {
            endpoint: endpoint.into(),
            config,
        }
    }
}
pub struct TimesFeedback {
    // all used i64 for ms
    pub t_net: i64,
    pub t_queue: i64,
    pub t_work: i64,
    pub t_asset: i64,
}

pub struct LoadBalancer<P: LoadBalancerPolicy> {
    workers: Vec<OffloadSite>, //vec of endpoint
    policy: Arc<P>,
}

pub struct SelectionGuard<'a, P: LoadBalancerPolicy> {
    pub endpoint: &'a str,
    worker_idx: usize,
    policy: Arc<P>,
    completed: bool,
}

impl<P: LoadBalancerPolicy> LoadBalancer<P> {
    pub fn new(addrs: impl IntoIterator<Item = OffloadSite>) -> Self {
        let addrs: Vec<OffloadSite> = addrs.into_iter().collect();
        let policy = P::new(&addrs);

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
            endpoint: &self.workers[idx].endpoint,
            worker_idx: idx,
            policy: Arc::clone(&self.policy),
            completed: false,
        }
    }
}

impl<P: LoadBalancerPolicy> SelectionGuard<'_, P> {
    pub fn finish(mut self, feedback: Option<&TimesFeedback>) {
        self.policy.complete(self.worker_idx, feedback);
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

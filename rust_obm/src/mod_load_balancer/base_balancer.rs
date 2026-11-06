// use std::net::ToSocketAddrs;
use std::future::Future;
use std::sync::Arc;

pub trait LoadBalancerPolicy: Send + Sync {
    // What select hands to complete for the same request (e.g. MCT's predicted cost), () if nothing.
    // Associated type defaults are unstable, so every policy names it
    type Ticket: Send;
    fn new(offloads: &[OffloadSite]) -> Self;
    fn select(&self, key: Option<&str>, workers: &[OffloadSite]) -> (usize, Self::Ticket);
    // For a policy that may hold a request until a site is free (e.g. Throttled); the others
    // decide at once. impl Future + Send, not async fn: the main server spawns the request task
    fn select_wait(
        &self,
        key: Option<&str>,
        workers: &[OffloadSite],
    ) -> impl Future<Output = (usize, Self::Ticket)> + Send {
        std::future::ready(self.select(key, workers))
    }
    fn complete(
        &self,
        _worker_idx: usize,
        _ticket: Self::Ticket,
        _feedback: Option<&TimesFeedback>,
    ) {
    }
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
    pub name: String,
    pub config: config::Config,
}
impl OffloadSite {
    pub fn new(
        endpoint: impl Into<String>,
        name: impl Into<String>,
        config: config::Config,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            name: name.into(),
            config,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    pub name: &'a str,
    pub config: config::Config,
    worker_idx: usize,
    policy: Arc<P>,
    ticket: Option<P::Ticket>, // Some until the request is reported to the policy
}

impl<P: LoadBalancerPolicy> LoadBalancer<P> {
    pub fn new(addrs: impl IntoIterator<Item = OffloadSite>) -> Self {
        let addrs: Vec<OffloadSite> = addrs.into_iter().collect();
        let policy = P::new(&addrs);
        Self::with_policy(addrs, policy)
    }
    // For a policy that needs more than the site list (e.g. MCT's cost predictor);
    // the policy must have been built from the same sites, in the same order
    pub fn with_policy(addrs: impl IntoIterator<Item = OffloadSite>, policy: P) -> Self {
        let addrs: Vec<OffloadSite> = addrs.into_iter().collect();

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
        let (idx, ticket) = self.policy.select(key, &self.workers);
        self.guard(idx, ticket)
    }
    // Same as select, but waits if the policy holds the request (main server path)
    pub async fn select_wait(&self, key: Option<&str>) -> SelectionGuard<'_, P> {
        let (idx, ticket) = self.policy.select_wait(key, &self.workers).await;
        self.guard(idx, ticket)
    }
    fn guard(&self, idx: usize, ticket: P::Ticket) -> SelectionGuard<'_, P> {
        SelectionGuard {
            endpoint: &self.workers[idx].endpoint,
            name: &self.workers[idx].name,
            config: self.workers[idx].config,
            worker_idx: idx,
            policy: Arc::clone(&self.policy),
            ticket: Some(ticket),
        }
    }
}

impl<P: LoadBalancerPolicy> SelectionGuard<'_, P> {
    pub fn finish(mut self, feedback: Option<&TimesFeedback>) {
        if let Some(ticket) = self.ticket.take() {
            self.policy.complete(self.worker_idx, ticket, feedback);
        }
    }
}

impl<P: LoadBalancerPolicy> Drop for SelectionGuard<'_, P> {
    fn drop(&mut self) {
        // still Some -> finish was never called (early return / error)
        if let Some(ticket) = self.ticket.take() {
            self.policy.complete(self.worker_idx, ticket, None);
        }
    }
}

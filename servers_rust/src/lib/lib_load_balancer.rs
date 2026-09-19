use std::net::SocketAddr;

use super::lib_conn_pool as conn_pool;

pub struct LoadBalancer {
    workers: Vec<conn_pool::TcpPool>,
}

impl LoadBalancer {
    pub fn with_workers(addr_slice: &[&str], max_conn_per_worker: usize) -> Option<LoadBalancer> {
        if addr_slice.is_empty() {
            return None;
        }

        let workers = addr_slice
            .iter()
            .map(|&addr| conn_pool::create_tcp_pool(addr, max_conn_per_worker))
            .collect();

        Some(LoadBalancer { workers })
    }
}

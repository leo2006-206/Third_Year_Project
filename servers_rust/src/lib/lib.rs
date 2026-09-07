// src/lib/lib.rs

pub mod lib_conn_pool;
pub mod lib_http;
pub mod lib_util;

// Optional: re-export directly so callers don't have to write server_util::...
pub use lib_conn_pool::{TcpConnection, TcpPool, create_tcp_pool};
pub use lib_http::*;
pub use lib_util::*;

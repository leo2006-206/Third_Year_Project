use std::io;

use deadpool::managed::{Manager, Metrics, Object, Pool, RecycleError, RecycleResult};
use smol::net::TcpStream;

/// Manager responsible for creating and validating pooled TCP connections
pub struct TcpManager {
    addr: String,
}

impl TcpManager {
    pub fn new(addr: impl Into<String>) -> Self {
        Self { addr: addr.into() }
    }
}

impl Manager for TcpManager {
    type Type = TcpStream;
    type Error = io::Error;

    /// Creates a new TCP connection and optimizes it for low latency
    async fn create(&self) -> Result<TcpStream, io::Error> {
        let stream = TcpStream::connect(&self.addr).await?;

        // Disable Nagle's algorithm for low-latency HTTP forwarding
        let _ = stream.set_nodelay(true);

        Ok(stream)
    }

    /// Validates if an idle connection in the pool is still alive before reusing
    async fn recycle(&self, conn: &mut TcpStream, _: &Metrics) -> RecycleResult<io::Error> {
        let mut buf = [0u8; 1];

        // Non-blocking peek directly on the underlying socket
        match conn.peek(&mut buf).await {
            // EOF: Remote closed the connection
            Ok(0) => Err(RecycleError::Backend(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "Connection closed by remote peer",
            ))),
            // Dirty state: Previous request left unread trailing bytes in buffer
            Ok(_) => Err(RecycleError::Backend(io::Error::new(
                io::ErrorKind::InvalidData,
                "Socket contains unread trailing bytes",
            ))),
            // WouldBlock: Socket is alive, open, and waiting for next command
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            // Any other OS network error
            Err(e) => Err(RecycleError::Backend(e)),
        }
    }
}

pub type TcpPool = Pool<TcpManager>;
pub type TcpConnection = Object<TcpManager>;

/// Builds a pool with maximum size and the smol-2 runtime
pub fn create_tcp_pool(addr: &str, max_size: usize) -> TcpPool {
    let mgr = TcpManager::new(addr);
    Pool::builder(mgr)
        .max_size(max_size)
        .runtime(deadpool::Runtime::Smol2)
        .build()
        .expect("Failed to build TCP connection pool")
}

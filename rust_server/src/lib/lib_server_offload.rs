use smol::net::TcpStream;
use std::time::Instant;

// One render request waiting for Dana. The Dana worker takes them one at a time in arrival
// order (FIFO channel, single receiver) and answers on `client` itself
pub struct DanaJob {
    pub client: TcpStream,
    pub path: String,
    pub enqueued: Instant, // for X-Queue-Time: time spent waiting in the queue
}

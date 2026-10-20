use smol::lock::{Semaphore, SemaphoreGuard};

static GATE: Semaphore = Semaphore::new(1);

pub async fn dana_gate_acquire() -> SemaphoreGuard<'static> {
    GATE.acquire().await
}

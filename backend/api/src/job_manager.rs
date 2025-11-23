use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct JobManager {
    semaphore: Arc<Semaphore>,
}

impl JobManager {
    pub fn new(limit: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(limit)),
        }
    }

    pub async fn acquire(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.semaphore
            .acquire()
            .await
            .expect("Semaphore closed unexpectedly")
    }
}

//! Own admitted suggestion work through cancellation and drain it before shutdown.
use crate::ServiceError;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;

#[derive(Debug)]
pub(in crate::pulls) struct Work {
    closed: Mutex<bool>,
    slots: Arc<Semaphore>,
    tasks: TaskTracker,
}
impl Default for Work {
    fn default() -> Self {
        Self { closed: Mutex::new(false), slots: Arc::new(Semaphore::new(2)), tasks: TaskTracker::new() }
    }
}
impl Work {
    pub fn submit<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Result<tokio::task::JoinHandle<T>, ServiceError> {
        let closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *closed {
            return Err(ServiceError::Unavailable { retry_after_seconds: 1 });
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })?;
        Ok(self.tasks.spawn(async move {
            let _permit = permit;
            future.await
        }))
    }
    pub async fn shutdown(&self) {
        {
            let mut closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *closed = true;
            self.slots.close();
            self.tasks.close();
        }
        self.tasks.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn review_workflow_cancelled_waiters_keep_admission_until_shutdown_drains() {
        let work = Arc::new(Work::default());
        let worker = work.clone();
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            worker
                .submit(async move {
                    let _ = started.send(());
                    let _ = resume.await;
                })
                .unwrap()
                .await
        });
        observed.await.unwrap();
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(work.slots.available_permits(), 1);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(10), work.shutdown()).await.is_err());
        assert!(work.submit(async {}).is_err());
        release.send(()).unwrap();
        work.shutdown().await;
        assert_eq!(work.slots.available_permits(), 2);
    }
}

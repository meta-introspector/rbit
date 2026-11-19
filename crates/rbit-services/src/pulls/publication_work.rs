//! Admitted publications outlive their caller and drain before stores close.
use crate::ServiceError;
use std::{
    future::Future,
    sync::{Arc, Mutex, PoisonError},
};
use tokio::{sync::Semaphore, task::JoinHandle};
use tokio_util::task::TaskTracker;

#[derive(Debug)]
pub(super) struct Work {
    slots: Arc<Semaphore>,
    closed: Mutex<bool>,
    tasks: TaskTracker,
}
impl Work {
    pub(super) fn new(capacity: usize) -> Self {
        Self { slots: Arc::new(Semaphore::new(capacity.max(1))), closed: Mutex::new(false), tasks: TaskTracker::new() }
    }
    pub(super) fn submit<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + Send + 'static,
    ) -> Result<JoinHandle<T>, ServiceError> {
        let closed = self.closed.lock().unwrap_or_else(PoisonError::into_inner);
        if *closed {
            return Err(ServiceError::Unavailable { retry_after_seconds: 1 });
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })?;
        Ok(self.tasks.spawn(async move {
            let _permit = permit;
            future.await
        }))
    }
    pub(super) async fn shutdown(&self) {
        {
            let mut closed = self.closed.lock().unwrap_or_else(PoisonError::into_inner);
            *closed = true;
            self.slots.close();
            self.tasks.close();
        }
        self.tasks.wait().await;
    }
}

impl super::PullService {
    /// Merges the pull request as `actor` (see the module docs). Returns the new tip of
    /// the base branch.
    pub async fn merge(
        &self,
        actor: &rbit_core::User,
        ctx: &super::PullContext,
        input: super::MergeInput,
    ) -> Result<String, super::MergeFailure> {
        let service = self.clone();
        let actor = actor.clone();
        let ctx = ctx.clone();
        self.inner
            .publication_work
            .submit(async move { service.merge_owned(&actor, &ctx, input).await })?
            .await
            .map_err(ServiceError::internal)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn security_publication_cancellation_retains_admission_and_shutdown_drains() {
        let work = Arc::new(Work::new(1));
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let job = work
            .submit(async move {
                entered.send(()).unwrap();
                held.await.unwrap();
            })
            .unwrap();
        started.await.unwrap();
        drop(job);
        assert!(work.submit(async {}).is_err(), "a cancelled waiter cannot release the publication slot");
        let shutdown = work.shutdown();
        tokio::pin!(shutdown);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut shutdown).await.is_err());
        release.send(()).unwrap();
        shutdown.await;
        assert!(work.submit(async {}).is_err(), "shutdown closes admission permanently");
    }
}

//! The owned parser retains bounded admission after a cancelled request and during shutdown.
use super::{MAX_REPORT_BYTES, SecurityService};
use crate::ServiceError;
use rbit_collaboration::security::Report;
use sha2::{Digest, Sha256};
impl SecurityService {
    pub(super) async fn parse(&self, bytes: Vec<u8>) -> Result<Report, ServiceError> {
        if bytes.len() > MAX_REPORT_BYTES {
            return Err(ServiceError::Invalid("SARIF exceeds 1 MiB.".into()));
        }
        let job = {
            let closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if *closed {
                return Err(ServiceError::Unavailable { retry_after_seconds: 1 });
            }
            let permit = self.slots.clone().try_acquire_owned().map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })?;
            self.tasks.spawn_blocking(move || {
                let _permit = permit;
                let mut report = rbit_collaboration::security::parse(&bytes).map_err(|error| ServiceError::Invalid(error.into()))?;
                for finding in &mut report.findings {
                    finding.identity = Sha256::digest(finding.identity.as_bytes())
                        .iter()
                        .flat_map(|byte| {
                            [char::from(b"0123456789abcdef"[(byte >> 4) as usize]), char::from(b"0123456789abcdef"[(byte & 15) as usize])]
                        })
                        .collect();
                }
                Ok::<_, ServiceError>(report)
            })
        };
        job.await.map_err(ServiceError::internal)?
    }
    pub async fn shutdown(&self) {
        self.background.stop.cancel();
        {
            let mut closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *closed = true;
            self.slots.close();
            self.tasks.close();
        }
        self.tasks.wait().await;
    }
}

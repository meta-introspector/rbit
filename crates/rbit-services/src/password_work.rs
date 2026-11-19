//! Password capacity is instance-owned, validated at composition and independent of throttles.

use rbit_core::Config;
use rbit_identity::PasswordWork;
use rbit_kernel::BlockingBudget;

use crate::ServiceError;

pub(crate) fn configured(config: &Config) -> Result<PasswordWork, ServiceError> {
    let capacity = config.security.password_job_capacity().map_err(ServiceError::Invalid)?;
    Ok(PasswordWork::new(BlockingBudget::new(capacity)))
}

/// An injected owner must enforce the configured capacity before any services are published.
pub(crate) fn validate_capacity(config: &Config, passwords: &PasswordWork) -> Result<(), ServiceError> {
    let capacity = config.security.password_job_capacity().map_err(ServiceError::Invalid)?;
    if passwords.capacity() != capacity {
        return Err(ServiceError::Invalid("password work capacity must match security.max_password_jobs".into()));
    }
    Ok(())
}

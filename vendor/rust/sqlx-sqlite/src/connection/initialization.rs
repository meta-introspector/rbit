//! Publish successful native initialization before any driver connection opens.
use std::ffi::c_int;
use std::sync::{Mutex, OnceLock};

use libsqlite3_sys::{sqlite3_initialize, SQLITE_OK};

use crate::{error::Error, SqliteError};

struct Initialization {
    success: OnceLock<()>,
    attempt: Mutex<()>,
}

impl Initialization {
    const fn new() -> Self {
        Self {
            success: OnceLock::new(),
            attempt: Mutex::new(()),
        }
    }

    fn ensure(&self, initialize: impl FnOnce() -> c_int) -> Result<(), Error> {
        if self.success.get().is_some() {
            return Ok(());
        }
        let _attempt = self
            .attempt
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.success.get().is_some() {
            return Ok(());
        }
        let code = initialize();
        if code != SQLITE_OK {
            return Err(Error::Database(Box::new(SqliteError::from_code(code))));
        }
        // OnceLock publishes successful native initialization to every fast reader.
        let _ = self.success.set(());
        Ok(())
    }
}

pub(super) fn ensure() -> Result<(), Error> {
    static INITIALIZATION: Initialization = Initialization::new();
    INITIALIZATION.ensure(|| {
        // SAFETY: driver attempts are serialized until success, and every native
        // connection open passes this boundary. This driver never calls shutdown.
        unsafe { sqlite3_initialize() }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Barrier,
    };

    #[test]
    fn concurrent_first_attempt_initializes_once_then_uses_success() {
        let state = Initialization::new();
        let calls = AtomicUsize::new(0);
        let start = Barrier::new(32);
        std::thread::scope(|threads| {
            for _ in 0..32 {
                threads.spawn(|| {
                    start.wait();
                    state
                        .ensure(|| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::yield_now();
                            SQLITE_OK
                        })
                        .unwrap();
                });
            }
        });
        state
            .ensure(|| panic!("successful initialization must not run again"))
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn native_error_is_preserved_and_next_attempt_can_recover() {
        let state = Initialization::new();
        let error = state.ensure(|| libsqlite3_sys::SQLITE_NOMEM).unwrap_err();
        let Error::Database(error) = error else {
            panic!("native error lost its database classification")
        };
        assert_eq!(error.code().as_deref(), Some("7"));
        assert!(!error.message().is_empty());
        state.ensure(|| SQLITE_OK).unwrap();
        state
            .ensure(|| panic!("recovered initialization must remain published"))
            .unwrap();
    }
}

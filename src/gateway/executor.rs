//! Bounded blocking executor for MT5 gateway operations.
//!
//! Offloads blocking I/O and DLL calls onto Tokio's blocking threadpool (`spawn_blocking`)
//! with concurrency bounded by a semaphore and deadlined by an explicit timeout.

use crate::error::{Mt5Error, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Bounded blocking executor that runs synchronous backend/DLL operations on Tokio's blocking threadpool.
#[derive(Clone, Debug)]
pub struct BlockingExecutor {
    permits: Arc<Semaphore>,
}

impl BlockingExecutor {
    /// Create a new blocking executor with `permits` maximum concurrent blocking tasks.
    pub fn new(permits: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(permits)),
        }
    }

    /// Execute a blocking operation with deadline and bounded concurrency.
    pub async fn run<F, T>(&self, timeout: Duration, op: &'static str, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        match tokio::time::timeout(timeout, tokio::task::spawn_blocking(f)).await {
            Ok(joined) => joined.map_err(|e| Mt5Error::JoinError(e.to_string()))?,
            Err(_) => Err(Mt5Error::Timeout {
                operation: op.to_string(),
            }),
        }
    }
}

impl Default for BlockingExecutor {
    fn default() -> Self {
        Self::new(32)
    }
}

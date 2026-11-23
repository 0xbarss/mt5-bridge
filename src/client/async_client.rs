//! Async facade for [`Mt5Client`].
//!
//! Wraps each blocking sync call in [`tokio::task::spawn_blocking`] via the bounded
//! [`BlockingExecutor`], so callers can use `client.account_info().await` without
//! blocking the Tokio runtime.
//!
//! # Serialized pipe
//!
//! The underlying pipe is serialized: even though this facade is async, concurrent
//! async calls are offloaded to the blocking pool and still serialize on the
//! single global pipe mutex inside the DLL. This facade improves responsiveness,
//! not throughput.

use crate::client::Mt5Client;
use crate::error::Result;
use crate::gateway::executor::BlockingExecutor;
use crate::types::{AccountInfo, Bar, Timeframe};
use std::sync::Arc;
use std::time::Duration;

/// Async wrapper around [`Mt5Client`] for users who want `async` APIs.
///
/// Each synchronous method is wrapped in [`tokio::task::spawn_blocking`] via a
/// bounded [`BlockingExecutor`], so calling [`AsyncMt5Client::account_info`] or
/// [`AsyncMt5Client::copy_bars`] never blocks the Tokio runtime.
///
/// # Serialized pipe
///
/// The pipe itself remains serialized: all async calls serialize on the single
/// global pipe mutex in the underlying DLL. This facade improves responsiveness,
/// not throughput.
pub struct AsyncMt5Client {
    inner: Arc<Mt5Client>,
    pool: BlockingExecutor,
}

impl AsyncMt5Client {
    /// Connect to MetaTrader 5 using only a pipe authentication secret token
    /// (`InpPipeSecret`), wrapping [`Mt5Client::connect_with_secret`] in a
    /// blocking task.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Mt5Error::DllLoadError`] when no `mt5_bridge.dll` is available,
    /// [`crate::Mt5Error::AlreadyConnected`] when a client is already connected in this
    /// process, or any error from [`Mt5Client::connect_with_secret`].
    pub async fn connect_with_secret(secret: &str) -> Result<Self> {
        let pool = BlockingExecutor::default();
        let secret = secret.to_string();
        let client = pool
            .run(Duration::from_secs(30), "connect_with_secret", move || {
                Mt5Client::connect_with_secret(&secret)
            })
            .await?;
        Ok(Self {
            inner: Arc::new(client),
            pool,
        })
    }

    /// Query current account balance, equity, and margin information.
    ///
    /// Wraps [`Mt5Client::account_info`] in a blocking task.
    pub async fn account_info(&self) -> Result<AccountInfo> {
        let inner = Arc::clone(&self.inner);
        self.pool
            .run(Duration::from_secs(10), "account_info", move || {
                inner.account_info()
            })
            .await
    }

    /// Fetch historical OHLCV bars for `symbol` on `timeframe` between `from` and `to`
    /// (UTC seconds).
    ///
    /// Wraps [`Mt5Client::copy_bars`] in a blocking task.
    pub async fn copy_bars(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Bar>> {
        let inner = Arc::clone(&self.inner);
        let symbol = symbol.to_string();
        self.pool
            .run(Duration::from_secs(120), "copy_bars", move || {
                inner.copy_bars(&symbol, timeframe, from, to)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_with_secret_fails_without_dll() {
        match AsyncMt5Client::connect_with_secret("test-secret").await {
            Ok(_) => panic!("connect_with_secret must fail without a DLL"),
            Err(err) => {
                assert!(
                    matches!(
                        err,
                        crate::error::Mt5Error::InvalidDllPath { .. }
                            | crate::error::Mt5Error::DllLoadError { .. }
                    ),
                    "expected a connection error, got {err:?}"
                );
            }
        }
    }
}

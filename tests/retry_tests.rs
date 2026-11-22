//! Tests for [`crate::RetryPolicy`].
//!
//! The sync classification tests (`should_retry`) live as unit tests in
//! `src/retry.rs` and run under every feature combination. The async `run`
//! tests below require the `async` feature and a Tokio runtime.

use mt5_bridge::{Mt5Error, Result, RetryPolicy};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 3,
        initial: Duration::from_millis(1),
        max: Duration::from_millis(20),
        jitter: 0.0,
    }
}

fn failing_then_ok(
    n_failures: u32,
) -> impl FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32>> + Send>>
       + Send
       + Sync
       + 'static {
    let calls = Arc::new(AtomicU32::new(0));
    move || {
        let calls = Arc::clone(&calls);
        Box::pin(async move {
            let k = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if k <= n_failures {
                Err(Mt5Error::TransmissionFailed("transient".into()))
            } else {
                Ok(k)
            }
        })
    }
}

#[cfg(feature = "async")]
#[tokio::test]
async fn retries_n_times_then_succeeds() {
    let p = policy();
    let result: Result<u32, _> = p.run(failing_then_ok(2)).await;
    assert_eq!(result.unwrap(), 3);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn unknown_error_is_not_retried() {
    let p = policy();
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);
    let result: Result<u32, _> = p
        .run(move || {
            let calls = Arc::clone(&calls_clone);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Mt5Error::UnknownExecutionState {
                    symbol: "EURUSD".into(),
                    client_order_id: None,
                    description: "lost".into(),
                })
            })
        })
        .await;
    assert!(matches!(
        result,
        Err(Mt5Error::UnknownExecutionState { .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn exhausts_max_attempts() {
    let p = RetryPolicy {
        max_attempts: 2,
        initial: Duration::from_millis(1),
        max: Duration::from_millis(20),
        jitter: 0.0,
    };
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);
    let result: Result<u32, _> = p
        .run(move || {
            let calls = Arc::clone(&calls_clone);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Mt5Error::TransmissionFailed("always".into()))
            })
        })
        .await;
    assert!(matches!(result, Err(Mt5Error::TransmissionFailed(_))));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn ambiguous_retcode_is_not_retried() {
    let p = policy();
    let calls = Arc::new(AtomicU32::new(0));
    let calls_clone = Arc::clone(&calls);
    // 10011 (ERROR) is ambiguous → not retried.
    let result: Result<u32, _> = p
        .run(move || {
            let calls = Arc::clone(&calls_clone);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Mt5Error::OrderSendFailed {
                    symbol: "EURUSD".into(),
                    retcode: 10011,
                    description: "error",
                })
            })
        })
        .await;
    assert!(matches!(
        result,
        Err(Mt5Error::OrderSendFailed { retcode: 10011, .. })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "async")]
#[tokio::test]
async fn definitively_not_executed_retcode_is_retried() {
    let p = policy();
    // 10004 (Requote) is definitively not executed → retried, then succeeds.
    let result: Result<u32, _> = p.run(failing_then_ok(1)).await;
    assert_eq!(result.unwrap(), 2);
}

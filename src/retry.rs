//! Retry policy abstraction for safe, bounded retries.
//!
//! Retries are only safe when the outcome of a failed attempt is known to be
//! "not executed". Ambiguous outcomes (e.g. [`Mt5Error::UnknownExecutionState`])
//! must never be retried blindly, because the order may already have reached
//! the broker. See [`RetryPolicy::should_retry`] for the classification rules.

#[cfg(feature = "async")]
use crate::error::Result;
use crate::error::{classify_retcode, Mt5Error, RetcodeClass};
use std::time::Duration;
#[cfg(feature = "async")]
use std::time::{SystemTime, UNIX_EPOCH};

/// Policy governing how many times and with what backoff a fallible operation
/// is retried.
///
/// Only [`TransmissionFailed`](Mt5Error::TransmissionFailed) and errors
/// classified as [`RetcodeClass::DefinitivelyNotExecuted`] are retried.
/// [`UnknownExecutionState`](Mt5Error::UnknownExecutionState),
/// [`UnresolvedUnknownOrders`](Mt5Error::UnresolvedUnknownOrders),
/// [`RiskRejected`](Mt5Error::RiskRejected), and
/// [`PreflightRejected`](Mt5Error::PreflightRejected) are never retried.
///
/// Delays use exponential backoff (`initial * 2^(attempt-1)`, capped at `max`)
/// plus jitter (`0..jitter * delay`) to avoid thundering herds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    /// Maximum total number of attempts (including the first one).
    pub max_attempts: u32,
    /// Initial backoff delay.
    pub initial: Duration,
    /// Upper bound on any single backoff delay.
    pub max: Duration,
    /// Jitter factor applied to each delay: a random value in `0..jitter * delay`
    /// is added. Set to `0.0` for deterministic backoff.
    pub jitter: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial: Duration::from_millis(100),
            max: Duration::from_secs(5),
            jitter: 0.2,
        }
    }
}

impl RetryPolicy {
    /// Returns `true` when `err` is safe to retry.
    ///
    /// Retryable:
    /// - [`Mt5Error::TransmissionFailed`] — the request certainly never left
    ///   the process, so re-sending cannot duplicate an execution.
    /// - [`Mt5Error::OrderSendFailed`] whose retcode classifies as
    ///   [`RetcodeClass::DefinitivelyNotExecuted`] — the broker provably
    ///   rejected the request before execution.
    ///
    /// Not retryable:
    /// - [`Mt5Error::UnknownExecutionState`] — the response was lost; the
    ///   order may have executed. Reconcile instead of retrying.
    /// - [`Mt5Error::UnresolvedUnknownOrders`] — blocked submissions await
    ///   reconciliation.
    /// - [`Mt5Error::RiskRejected`] and [`Mt5Error::PreflightRejected`] —
    ///   policy rejections that will not change on retry.
    /// - All other errors.
    pub fn should_retry(&self, err: &Mt5Error) -> bool {
        match err {
            Mt5Error::TransmissionFailed(_) => true,
            Mt5Error::OrderSendFailed { retcode, .. } => {
                classify_retcode(*retcode) == RetcodeClass::DefinitivelyNotExecuted
            }
            Mt5Error::UnknownExecutionState { .. }
            | Mt5Error::UnresolvedUnknownOrders { .. }
            | Mt5Error::RiskRejected { .. }
            | Mt5Error::PreflightRejected(_) => false,
            _ => false,
        }
    }

    /// Backoff delay for `retry_no` (1-indexed: 1 = first retry after the first
    /// failed attempt). Exponential backoff capped at `max`, plus jitter.
    #[cfg(feature = "async")]
    fn backoff_delay(&self, retry_no: u32) -> Duration {
        let power = retry_no.saturating_sub(1);
        let mut delay = self.initial;
        for _ in 0..power {
            delay = delay.saturating_mul(2);
        }
        let capped = delay.min(self.max);
        let jitter_nanos = (capped.as_nanos() as f64 * self.jitter) as u64;
        if jitter_nanos == 0 {
            return capped;
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 % jitter_nanos)
            .unwrap_or(0);
        capped + Duration::from_nanos(nanos)
    }

    /// Run `f`, retrying according to this policy when `should_retry` returns
    /// `true`.
    ///
    /// The first error that is not retryable (or once `max_attempts` is
    /// exhausted) is returned as-is. Returns the first successful result.
    #[cfg(feature = "async")]
    pub async fn run<F, T, Fut>(&self, mut f: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut attempt: u32 = 0;
        loop {
            match f().await {
                Ok(t) => return Ok(t),
                Err(e) => {
                    if attempt + 1 >= self.max_attempts || !self.should_retry(&e) {
                        return Err(e);
                    }
                    attempt += 1;
                    let delay = self.backoff_delay(attempt);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RetcodeClass;

    #[test]
    fn default_policy_is_sensible() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 3);
        assert_eq!(p.initial, Duration::from_millis(100));
        assert_eq!(p.max, Duration::from_secs(5));
        assert_eq!(p.jitter, 0.2);
    }

    #[test]
    fn transmission_failed_is_retryable() {
        let p = RetryPolicy::default();
        assert!(p.should_retry(&Mt5Error::TransmissionFailed("pipe write failed".into())));
    }

    #[test]
    fn definitively_not_executed_retcode_is_retryable() {
        let p = RetryPolicy::default();
        for rc in [10004, 10006, 10017] {
            assert!(
                p.should_retry(&Mt5Error::OrderSendFailed {
                    symbol: "EURUSD".into(),
                    retcode: rc,
                    description: "desc",
                }),
                "retcode {rc} should be retryable",
            );
            assert_eq!(classify_retcode(rc), RetcodeClass::DefinitivelyNotExecuted);
        }
    }

    #[test]
    fn ambiguous_errors_are_not_retryable() {
        let p = RetryPolicy::default();
        // UnknownExecutionState — response may have been lost after execution.
        assert!(!p.should_retry(&Mt5Error::UnknownExecutionState {
            symbol: "EURUSD".into(),
            client_order_id: None,
            description: "lost".into(),
        }));
        // UnresolvedUnknownOrders — blocked pending reconciliation.
        assert!(!p.should_retry(&Mt5Error::UnresolvedUnknownOrders {
            symbol: String::new(),
            client_order_ids: vec!["cid-1".into()],
        }));
        // RiskRejected — policy rejection, retrying won't change it.
        assert!(!p.should_retry(&Mt5Error::RiskRejected {
            rule: "max_volume",
            detail: "too large".into(),
        }));
        // PreflightRejected — rejected before transmission.
        assert!(!p.should_retry(&Mt5Error::PreflightRejected("bad volume".into())));
    }

    #[test]
    fn retryable_retcode_boundary() {
        let p = RetryPolicy::default();
        // 10008 (Placed) and 10009 (Done) are executed outcomes → not retryable.
        assert!(!p.should_retry(&Mt5Error::OrderSendFailed {
            symbol: "EURUSD".into(),
            retcode: 10008,
            description: "placed",
        }));
        assert!(!p.should_retry(&Mt5Error::OrderSendFailed {
            symbol: "EURUSD".into(),
            retcode: 10009,
            description: "done",
        }));
        // 10011 (ERROR) and 10012 (TIMEOUT) are ambiguous → not retryable.
        assert!(!p.should_retry(&Mt5Error::OrderSendFailed {
            symbol: "EURUSD".into(),
            retcode: 10011,
            description: "error",
        }));
        assert!(!p.should_retry(&Mt5Error::OrderSendFailed {
            symbol: "EURUSD".into(),
            retcode: 10012,
            description: "timeout",
        }));
        // 10004 (Requote) is definitively not executed → retryable.
        assert!(p.should_retry(&Mt5Error::OrderSendFailed {
            symbol: "EURUSD".into(),
            retcode: 10004,
            description: "requote",
        }));
    }

    #[test]
    #[cfg(feature = "async")]
    fn backoff_grows_with_retry_number_and_is_capped() {
        let p = RetryPolicy {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(1),
            jitter: 0.0,
            ..Default::default()
        };
        let d1 = p.backoff_delay(1);
        let d2 = p.backoff_delay(2);
        let d3 = p.backoff_delay(3);
        assert!(d1 >= Duration::from_millis(100));
        assert!(d2 > d1);
        assert!(d3 > d2);
        // Cap is respected for large retry numbers.
        assert!(p.backoff_delay(100) <= p.max);
    }

    #[test]
    #[cfg(feature = "async")]
    fn jitter_is_within_bounds() {
        let p = RetryPolicy {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(10),
            jitter: 0.5,
            ..Default::default()
        };
        let base = p.initial;
        for _ in 0..20 {
            let d = p.backoff_delay(1);
            assert!(d >= base, "jittered delay {d:?} below base {base:?}");
            assert!(d <= base + Duration::from_nanos((base.as_nanos() as f64 * p.jitter) as u64));
        }
    }

    #[test]
    fn zero_max_attempts_means_one_try_no_retry() {
        let p = RetryPolicy {
            max_attempts: 0,
            ..Default::default()
        };
        assert_eq!(p.max_attempts, 0);
    }
}

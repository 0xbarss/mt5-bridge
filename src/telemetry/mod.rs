//! Telemetry / metrics export for bridge observability.
//!
//! Gated on the `metrics` feature. Backed by the `prometheus` crate.
//! Metrics are registered against a global process-wide registry and can be
//! exported in prometheus text format via [`crate::telemetry::prometheus::encode`].

use ::prometheus::{exponential_buckets, Histogram, HistogramOpts, IntCounter, IntGauge, Registry};
use std::sync::OnceLock;

pub mod prometheus;

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Global prometheus registry shared by all metric handles.
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::new)
}

/// Pre-registered counter for total bridge requests sent to the MT5 EA.
pub fn requests_total() -> &'static IntCounter {
    static HANDLE: OnceLock<IntCounter> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let c = IntCounter::new(
            "mt5_bridge_requests_total",
            "Total requests sent to the MT5 EA",
        )
        .expect("valid metric definition");
        registry()
            .register(Box::new(c.clone()))
            .expect("metric registration");
        c
    })
}

/// Pre-registered counter for total events dropped by the bridge (all types).
pub fn drops_total() -> &'static IntCounter {
    static HANDLE: OnceLock<IntCounter> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let c = IntCounter::new(
            "mt5_bridge_drops_total",
            "Total events dropped by the bridge",
        )
        .expect("valid metric definition");
        registry()
            .register(Box::new(c.clone()))
            .expect("metric registration");
        c
    })
}

/// Pre-registered gauge for current event queue depth.
pub fn queue_depth() -> &'static IntGauge {
    static HANDLE: OnceLock<IntGauge> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let g = IntGauge::new("mt5_bridge_queue_depth", "Current event queue depth")
            .expect("valid metric definition");
        registry()
            .register(Box::new(g.clone()))
            .expect("metric registration");
        g
    })
}

/// Pre-registered histogram for request round-trip latency in microseconds.
pub fn request_latency_histogram() -> &'static Histogram {
    static HANDLE: OnceLock<Histogram> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let buckets = exponential_buckets(100.0, 2.0, 25).expect("valid latency buckets");
        let h = Histogram::with_opts(
            HistogramOpts::new(
                "mt5_bridge_request_latency_us",
                "Request round-trip latency in microseconds",
            )
            .buckets(buckets),
        )
        .expect("valid histogram definition");
        registry()
            .register(Box::new(h.clone()))
            .expect("metric registration");
        h
    })
}

/// Record a request latency observation (microseconds).
pub fn record_request_latency_us(us: u64) {
    request_latency_histogram().observe(us as f64);
}

/// Compute the p50/p95/p99 request latency percentiles from the histogram.
///
/// Returns `(0, 0, 0)` when no observations have been recorded.
pub fn latency_percentiles() -> (u64, u64, u64) {
    let families = registry().gather();
    let payload = families
        .iter()
        .find(|f| f.get_name() == "mt5_bridge_request_latency_us")
        .and_then(|f| f.get_metric().first())
        .map(|m| m.get_histogram());
    let Some(payload) = payload else {
        return (0, 0, 0);
    };
    let total = payload.get_sample_count();
    if total == 0 {
        return (0, 0, 0);
    }
    let buckets = payload.get_bucket();
    let percentile = |p: f64| -> u64 {
        let target = (p / 100.0) * total as f64;
        let mut prev_upper = 0.0f64;
        let mut prev_count = 0u64;
        for bucket in buckets {
            let count = bucket.get_cumulative_count();
            if count as f64 >= target {
                let upper = bucket.get_upper_bound();
                if upper.is_infinite() {
                    return prev_upper.max(0.0) as u64;
                }
                // Linear interpolation within the bucket.
                let span = upper - prev_upper;
                let count_span = count - prev_count;
                let frac = if count_span > 0 && span > 0.0 {
                    ((target - prev_count as f64) / count_span as f64).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                return (prev_upper + frac * span).max(0.0) as u64;
            }
            prev_upper = bucket.get_upper_bound();
            prev_count = count;
        }
        0
    };
    (percentile(50.0), percentile(95.0), percentile(99.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_shared() {
        assert!(std::ptr::eq(registry(), registry()));
    }

    #[test]
    fn requests_total_increments_and_persists() {
        let c = requests_total();
        c.inc();
        c.inc();
        c.inc();
        assert_eq!(c.get(), 3);
        // The same handle is returned on subsequent calls.
        assert!(std::ptr::eq(c, requests_total()));
    }

    #[test]
    fn queue_depth_gauge_sets_value() {
        let g = queue_depth();
        g.set(42);
        assert_eq!(g.get(), 42);
    }

    #[test]
    fn latency_percentiles_are_ordered_when_empty() {
        let (p50, p95, p99) = latency_percentiles();
        assert!(p50 <= p95 && p95 <= p99);
    }

    #[test]
    fn latency_percentiles_record_and_compute() {
        record_request_latency_us(100);
        record_request_latency_us(200);
        record_request_latency_us(300);
        record_request_latency_us(400);
        record_request_latency_us(500);
        let (p50, p95, p99) = latency_percentiles();
        assert!(p50 > 0, "p50 should be positive with observations");
        assert!(p95 >= p50, "p95 should be >= p50");
        assert!(p99 >= p95, "p99 should be >= p95");
    }

    #[test]
    fn request_latency_histogram_is_shared() {
        assert!(std::ptr::eq(
            request_latency_histogram(),
            request_latency_histogram()
        ));
    }
}

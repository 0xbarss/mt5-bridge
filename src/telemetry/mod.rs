//! Telemetry / metrics export for bridge observability.
//!
//! Gated on the `metrics` feature. Backed by the `prometheus` crate.
//! Metrics are registered against a global process-wide registry and can be
//! exported in prometheus text format via [`crate::telemetry::prometheus::encode`].

use prometheus::{IntCounter, IntGauge, Registry};
use std::sync::OnceLock;

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
}

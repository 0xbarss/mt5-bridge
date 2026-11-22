//! Prometheus text-format encoding for bridge metrics.
//!
//! Encodes all registered metrics in the prometheus exposition text format
//! (version 0.0.4).

use crate::telemetry::registry;
use prometheus::TextEncoder;

/// HTTP content type for prometheus metrics exposition.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Encodes all registered metrics in prometheus text format.
///
/// Suitable for serving over HTTP at a `/metrics` endpoint.
pub fn encode() -> Result<String, prometheus::Error> {
    let encoder = TextEncoder::new();
    let metric_families = registry().gather();
    encoder.encode_to_string(&metric_families)
}

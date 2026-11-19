use serde::{Deserialize, Serialize};

/// Mode for market data tick and event streaming (protocol v5+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StreamMode {
    /// Execution stream: low-latency bounded queue that drops stale quotes when the consumer lags.
    #[default]
    Latest,
    /// Recorder stream: lossless delivery that preserves every event, causing backpressure if consumer is slow.
    Lossless,
}

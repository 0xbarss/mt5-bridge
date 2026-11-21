//! MT5 Gateway Actor with priority queues, bounded blocking execution, and event streaming.
//!
//! Separates execution commands from market data streams:
//! - Execution commands run via a bounded [`BlockingExecutor`], ensuring the Tokio runtime is never blocked by synchronous DLL pipe I/O.
//! - Three-tier priority message scheduling:
//!   - Priority 1 (Latency-Critical): Market orders, order modifications, emergency closes. High-priority commands preempt in-flight P3 bulk downloads.
//!   - Priority 2 (Transactional): Account info, positions, pending orders, deals, manual reconciliation, and responsiveness pings.
//!   - Priority 3 (Bulk): Historical candle downloads (`copy_rates_chunked`).
//! - Starvation-free priority scheduling ensuring P1 commands are never blocked by heavy P3 downloads.
//! - Lossless trade event stream with `event_seq` gap detection triggering immediate reconciliation.
//! - Tick conflation via Tokio `watch` channels to decouple order execution from high-frequency tick bursts.

pub mod actor;
pub mod executor;

pub use actor::{Mt5Gateway, Mt5GatewayHandle, P1Command, P2Command, P3Command};
pub use executor::BlockingExecutor;

use crate::session::SessionConfig;
use std::time::Duration;

/// Configuration options for the MT5 Gateway actor and its blocking executor.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Session configuration for MT5 lifecycle and reconnect backoff.
    pub session: SessionConfig,
    /// Maximum concurrent blocking operations allowed simultaneously in [`BlockingExecutor`].
    pub executor_permits: usize,
    /// Deadline for Priority 1 latency-critical operations.
    pub p1_timeout: Duration,
    /// Deadline for Priority 2 transactional operations.
    pub p2_timeout: Duration,
    /// Deadline for Priority 3 bulk download operations.
    pub p3_timeout: Duration,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            session: SessionConfig::default(),
            executor_permits: 32,
            p1_timeout: Duration::from_secs(30),
            p2_timeout: Duration::from_secs(10),
            p3_timeout: Duration::from_secs(120),
        }
    }
}

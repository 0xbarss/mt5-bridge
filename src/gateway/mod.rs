//! MT5 Gateway Actor with priority queues, bounded blocking execution, and event streaming.
//!
//! Separates execution commands from market data streams:
//! - Execution commands run via a bounded [`BlockingExecutor`], ensuring the Tokio runtime is never blocked by synchronous DLL pipe I/O.
//! - Three-tier priority message scheduling:
//!   - Priority 1 (Latency-Critical): Market orders, order modifications, emergency closes. High-priority commands preempt in-flight P3 bulk downloads.
//!   - Priority 2 (Transactional): Account info, positions, pending orders, deals, manual reconciliation, and responsiveness pings.
//!   - Priority 3 (Bulk): Historical candle downloads (`copy_rates_chunked`).
//! - Starvation-free priority scheduling ensuring P1 commands are never blocked by heavy P3 downloads.
//! - Bounded, ordered trade event fan-out with per-subscriber lag detection via
//!   [`TradeSubscription::recv_checked`](crate::stream::TradeSubscription::recv_checked); `event_seq`
//!   gap detection triggers immediate reconciliation.
//! - Tick conflation via Tokio `watch` channels to decouple order execution from high-frequency tick bursts.

pub mod actor;
pub mod executor;

pub use actor::{Mt5Gateway, Mt5GatewayHandle, P1Command, P2Command, P3Command};
pub use executor::BlockingExecutor;

use crate::backend::TradingBackend;
use crate::error::{Mt5Error, Result};
use crate::risk::RiskEngine;
use crate::session::SessionConfig;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Abstraction for connecting and health-checking trading backends with automatic lifecycle supervision.
pub trait Connector<B: TradingBackend + Send + Sync + 'static>: Send + Sync + 'static {
    /// Establish a fresh connection to the trading backend.
    fn connect(&self) -> Result<B>;

    /// Check connection health of an active backend instance.
    fn health_check(&self, backend: &B) -> Result<()> {
        backend.account_info().map(|_| ())
    }
}

/// One-shot connector wrapping a pre-connected backend. Subsequent reconnect attempts fail.
pub struct FixedConnector<B> {
    backend: Mutex<Option<B>>,
}

impl<B> FixedConnector<B> {
    /// Create a new one-shot connector.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Mutex::new(Some(backend)),
        }
    }
}

impl<B: TradingBackend + Send + Sync + 'static> Connector<B> for FixedConnector<B> {
    fn connect(&self) -> Result<B> {
        self.backend
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or_else(|| {
                Mt5Error::PreflightRejected("one-shot backend cannot reconnect".to_string())
            })
    }
}

impl<B, F> Connector<B> for F
where
    B: TradingBackend + Send + Sync + 'static,
    F: Fn() -> Result<B> + Send + Sync + 'static,
{
    fn connect(&self) -> Result<B> {
        (self)()
    }
}

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
    /// Optional pre-trade risk engine. When set, every order submission is checked
    /// against portfolio-level limits before reaching the backend.
    pub risk_engine: Option<Arc<RiskEngine>>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            session: SessionConfig::default(),
            executor_permits: 32,
            p1_timeout: Duration::from_secs(30),
            p2_timeout: Duration::from_secs(10),
            p3_timeout: Duration::from_secs(120),
            risk_engine: None,
        }
    }
}

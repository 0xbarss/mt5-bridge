//! Pre-trade risk engine: portfolio-level checks before an order reaches the MT5 backend.
//!
//! [`RiskEngine`] evaluates every submission against [`RiskLimits`] (open positions,
//! per-symbol and total volume, daily loss, drawdown, symbol allow/block list, and
//! orders-per-minute). A rejection never reaches the backend.
//!
//! # Integration
//!
//! The gateway calls [`RiskEngine::pre_trade`] before
//! [`OrderManager::submit_order`](crate::reconciliation::OrderManager::submit_order).
//! Portfolio state is kept current via [`RiskEngine::on_account_info`],
//! [`RiskEngine::on_positions`], and [`RiskEngine::on_working_orders`].
//!
//! # Example
//!
//! ```no_run
//! use mt5_bridge::error::Result;
//! use mt5_bridge::risk::{RiskEngine, RiskLimits};
//! use mt5_bridge::types::{AccountInfo, OrderRequest, OrderType, Position, WorkingOrder};
//!
//! let engine = RiskEngine::new(RiskLimits {
//!     max_open_positions: 5,
//!     ..Default::default()
//! });
//! let req = OrderRequest::buy("EURUSD", 1.0);
//! let decision = engine.pre_trade(&req, &AccountInfo::default(), &[], &[])?;
//! assert_eq!(decision, mt5_bridge::risk::PreTradeDecision::Allowed);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod engine;
pub mod limits;

pub use engine::{PreTradeDecision, RiskEngine};
pub use limits::RiskLimits;

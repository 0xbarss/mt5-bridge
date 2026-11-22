//! Trade management: brackets, break-even, and trailing stops.
//!
//! [`TradeManager`](manager::TradeManager) subscribes to [`OrderEvent`]s emitted by
//! [`OrderManager`](crate::reconciliation::OrderManager) and maintains stop-loss /
//! take-profit rules that survive process restarts via the WAL.
//!
//! # Example
//!
//! ```no_run
//! use mt5_bridge::reconciliation::{OrderManager, SharedOrderManager, StrategyMagic};
//! use mt5_bridge::trade::{Bracket, TrailingStop, TradeManager};
//! use mt5_bridge::Mt5Client;
//! use std::sync::Arc;
//!
//! let manager = OrderManager::try_new(20240101).expect("valid magic");
//! let shared = SharedOrderManager::new(manager);
//! let client = Arc::new(Mt5Client::connect(12345678, "password", "Server")?);
//! let tm = TradeManager::new(shared.clone(), client);
//!
//! // The gateway or application loop drives `check_rules` on every tick:
//! // tm.check_rules("EURUSD", tick.bid)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod bracket;
pub mod manager;
pub mod trailing;

pub use bracket::Bracket;
pub use manager::TradeManager;
pub use trailing::TrailingStop;

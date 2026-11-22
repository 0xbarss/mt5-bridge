//! # mt5-bridge
//!
//! Fast, unofficial MetaTrader 5 API bridge and client for Rust.
//!
//! This library provides a clean, safe, and idiomatic Rust interface to interact with
//! an active MetaTrader 5 terminal. Because MetaQuotes only provides an official
//! Python API (with Windows-only DLL limitations), `mt5-bridge` fills the gap by providing
//! high-performance native integration for Rust and other systems languages.
//!
//! ## Architecture Overview
//!
//! ```text
//! ┌────────────────────────┐
//! │   Rust Application     │
//! │  (mt5-bridge client)   │
//! └───────────┬────────────┘
//!             │ FFI (libloading)
//! ┌───────────▼────────────┐
//! │     mt5_bridge.dll     │  (C++ Named Pipe Client)
//! └───────────┬────────────┘
//!             │ Windows Named Pipe (\\.\pipe\mt5bridge)
//! ┌───────────▼────────────┐
//! │    mt5_bridge.mq5      │  (MQL5 Expert Advisor Pipe Server)
//! └───────────┬────────────┘
//!             │ Internal API
//! ┌───────────▼────────────┐
//! │   MetaTrader 5 Client  │  (Live trading terminal)
//! └────────────────────────┘
//! ```
//!
//! ## Key Features
//!
//! - **Account Information**: Real-time balance, equity, margin, free margin, and floating PnL.
//! - **Symbol Specifications**: Point size, tick value, lot limits, spread, and price digits with in-memory caching.
//! - **Historical Data (OHLCV)**: Fetch bars across any standard timeframe (`M1` to `MN1`) with stabilization loops.
//! - **Real-time Streaming**: Non-blocking tick streaming and closed-bar streams via Tokio channels.
//! - **Live Trade Execution**: Instant market orders (`Buy`/`Sell`), pending orders (`Limit`/`Stop`), position closing, and SL/TP modification.
//! - **Zero Overhead**: Direct binary protocol over local IPC named pipe.
//!
//! ## Quick Example
//!
//! ```no_run
//! use mt5_bridge::{Mt5Client, Timeframe};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect to MT5 bridge
//!     let client = Mt5Client::connect(12345678, "account_password", "Broker-Demo")?;
//!
//!     // Check balance & equity
//!     let account = client.account_info()?;
//!     println!("Balance: {:.2}, Equity: {:.2}", account.balance, account.equity);
//!
//!     // Fetch latest 100 EURUSD M15 bars
//!     let now = chrono::Utc::now().timestamp();
//!     let bars = client.copy_bars("EURUSD", Timeframe::M15, now - 86400, now)?;
//!     println!("Fetched {} bars", bars.len());
//!
//!     Ok(())
//! }
//! ```

pub mod backend;
pub mod client;
pub mod constants;
pub mod error;
pub mod ffi;
pub mod journal;
pub mod position_book;
pub mod reconciliation;
pub mod risk;
pub mod session;
pub mod sync;
pub mod trade;
pub mod types;

#[cfg(feature = "wal")]
pub mod wal;

#[cfg(feature = "async")]
pub mod gateway;
#[cfg(feature = "async")]
pub mod stream;

pub use backend::TradingBackend;
pub use client::Mt5Client;
pub use constants::*;
pub use error::{classify_retcode, mt5_retcode_description, Mt5Error, Result, RetcodeClass};
pub use ffi::PROTOCOL_VERSION;
#[cfg(feature = "async")]
pub use gateway::{
    BlockingExecutor, Connector, FixedConnector, GatewayConfig, Mt5Gateway, Mt5GatewayHandle,
    P1Command, P2Command, P3Command,
};
#[cfg(feature = "json-journal")]
pub use journal::JsonFileStore;
pub use journal::OrderStore;
pub use position_book::{
    BookPosition, DealReason, PositionBook, PositionDeal, PositionRef, PositionState,
    PositionTransition,
};
pub use reconciliation::{
    BrokerSnapshot, LifecycleState, OrderEvent, OrderManager, OrderObserver, OrderSnapshot,
    ReconciliationReport, SafetyPolicy, SharedOrderManager, StrategyMagic, UnknownBlockScope,
};
pub use risk::{PreTradeDecision, RiskEngine, RiskLimits};
pub use session::{Mt5Session, SessionConfig};
pub use sync::LockExt;
pub use trade::{Bracket, TradeManager, TrailingStop};
pub use types::{
    calculate_sl_ticks, calculate_tp_ticks, comment_matches_client_order_id, parse_wire_id,
    price_to_ticks, ticks_to_price, truncate_utf8, wire_id, AccountInfo, AttributeMismatch, Bar,
    BookEvent, Deal, DealEntry, EventEffect, Fill, HistoryResult, Level, MismatchKind,
    OrderRequest, OrderState, OrderType, PingResult, Position, Rate, RejectionEvidence, StreamMode,
    SymbolInfo, Tick, Timeframe, TrackedOrder, TradeEvent, TradeResult, TradeRule, TradeStatus,
    WorkingOrder, MAX_CLIENT_ORDER_ID_BYTES, MT5_COMMENT_MAX_BYTES, WIRE_COMMENT_PREFIX,
    WIRE_ID_LEN,
};
#[cfg(all(feature = "wal", feature = "json-journal"))]
pub use wal::{migrate_json_to_wal, wal_to_json};
#[cfg(feature = "wal")]
pub use wal::{WalConfig, WalOrderStore};

#[cfg(feature = "async")]
pub use stream::{
    bar_end_time, stream_bars, stream_ticks, stream_ticks_with_config, BackpressurePolicy,
    BookSubscription, EventBus, SequenceTracker, StreamConfig, StreamError, TickSubscription,
    TradeSubscription,
};

#[cfg(doctest)]
doc_comment::doctest!("../README.md");

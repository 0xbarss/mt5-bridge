//! Market data helpers: bar aggregation, symbol discovery, and caching.
//!
//! This module provides tooling for deriving bars from a tick stream, for
//! discovering the MT5 symbol universe, and for caching recently fetched bars
//! so repeated `copy_bars` calls can be served from memory.
//!
//! - [`BarAggregator`] — consume [`Tick`]s and emit [`Bar`]s for time,
//!   tick-count, volume, and range bar kinds.
//! - [`BarCache`] — bounded, TTL-backed cache keyed by `(symbol, timeframe)`.
//! - [`SymbolRegistry`] — refreshable in-memory cache of MT5 symbols discovered
//!   through [`crate::Mt5Client::symbols`]/[`crate::Mt5Client::symbols_filtered`].

pub mod bars;
pub mod symbols;

pub use bars::{BarAggregator, BarCache, BarKind};
pub use symbols::SymbolRegistry;

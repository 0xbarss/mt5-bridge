//! Market data helpers: bar aggregation, symbol discovery, book snapshots, and caching.
//!
//! This module provides tooling for deriving bars from a tick stream, for
//! discovering the MT5 symbol universe, for assembling depth-of-book snapshots
//! from pushed book events, and for caching recently fetched bars so repeated
//! `copy_bars` calls can be served from memory.
//!
//! - [`BarAggregator`] — consume [`Tick`]s and emit [`Bar`]s for time,
//!   tick-count, volume, and range bar kinds.
//! - [`BarCache`] — bounded, TTL-backed cache keyed by `(symbol, timeframe)`.
//! - [`SymbolRegistry`] — refreshable in-memory cache of MT5 symbols discovered
//!   through [`crate::Mt5Client::symbols`]/[`crate::Mt5Client::symbols_filtered`].
//! - [`BookSnapshot`] / [`BookStream`] — assemble depth-of-book levels from
//!   [`crate::types::BookEvent`]s, with imbalance and weighted-mid metrics.

pub mod bars;
pub mod book;
pub mod symbols;

pub use bars::{BarAggregator, BarCache, BarKind};
pub use book::{BookLevel, BookSnapshot, BookStream};
pub use symbols::SymbolRegistry;

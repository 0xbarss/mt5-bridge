//! Market data helpers: bar aggregation and caching.
//!
//! This module provides tooling for deriving bars from a tick stream and for
//! caching recently fetched bars so repeated `copy_bars` calls can be served
//! from memory.
//!
//! - [`BarAggregator`] — consume [`Tick`]s and emit [`Bar`]s for time,
//!   tick-count, volume, and range bar kinds.
//! - [`BarCache`] — bounded, TTL-backed cache keyed by `(symbol, timeframe)`.

pub mod bars;

pub use bars::{BarAggregator, BarCache, BarKind};

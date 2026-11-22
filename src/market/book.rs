//! Depth-of-book snapshot assembly from [`BookEvent`]s.
//!
//! [`BookStream`] consumes [`BookEvent`]s from the push stream, maintains a
//! per-side (bid/ask) sorted level map, and periodically emits a
//! [`BookSnapshot`] with imbalance and weighted-mid metrics.

use crate::types::BookEvent;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// A single book level: price and resting volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookLevel {
    pub price: f64,
    pub volume: f64,
}

/// A point-in-time depth-of-book snapshot for a symbol.
#[derive(Debug, Clone, PartialEq)]
pub struct BookSnapshot {
    pub symbol: String,
    pub time_msc: i64,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
}

impl BookSnapshot {
    /// Top-N imbalance: `(bid_volume - ask_volume) / (bid_volume + ask_volume)`
    /// across the first `levels` levels of each side.
    ///
    /// Returns `0.0` when there is no volume on either side.
    pub fn imbalance(&self, levels: usize) -> f64 {
        let bid_vol: f64 = self.bids.iter().take(levels).map(|l| l.volume).sum();
        let ask_vol: f64 = self.asks.iter().take(levels).map(|l| l.volume).sum();
        let total = bid_vol + ask_vol;
        if total == 0.0 {
            0.0
        } else {
            (bid_vol - ask_vol) / total
        }
    }

    /// Volume-weighted mid price across the top level of each side.
    ///
    /// Each side's price is weighted by the opposing side's volume:
    /// `(best_bid.price * best_ask.volume + best_ask.price * best_bid.volume)
    /// / (best_bid.volume + best_ask.volume)`.
    ///
    /// Returns `0.0` when either side is empty.
    pub fn weighted_mid(&self) -> f64 {
        let Some(best_bid) = self.bids.first() else {
            return 0.0;
        };
        let Some(best_ask) = self.asks.first() else {
            return 0.0;
        };
        let total_vol = best_bid.volume + best_ask.volume;
        if total_vol == 0.0 {
            0.0
        } else {
            (best_bid.price * best_ask.volume + best_ask.price * best_bid.volume) / total_vol
        }
    }
}

/// Ordered wrapper around an `f64` price so it can be used as a `BTreeMap` key.
///
/// NaN compares as less than any non-NaN price, which is acceptable because
/// book prices from MT5 are never NaN.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PriceKey(f64);

// `f64` does not implement `Eq`/`Ord` (NaN != NaN), so both are implemented
// manually. MT5 book prices are never NaN, so the equivalence relation holds.
impl Eq for PriceKey {}

impl Ord for PriceKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.0).unwrap_or(std::cmp::Ordering::Less)
    }
}

impl PartialOrd for PriceKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Assembles book levels from [`BookEvent`]s and emits throttled snapshots.
///
/// Each [`BookEvent`] carries a single price level on either the bid side
/// (`is_buy == true`) or the ask side (`is_buy == false`). A volume of `0.0`
/// removes the level at that price.
///
/// Bids are kept sorted highest-first; asks lowest-first, matching the
/// conventional book display.
pub struct BookStream {
    symbol: String,
    bids: BTreeMap<Reverse<PriceKey>, f64>,
    asks: BTreeMap<PriceKey, f64>,
    last_emit: Option<Instant>,
    throttle: Duration,
}

impl BookStream {
    /// Create a new stream for `symbol` that emits at most one snapshot per
    /// `throttle` interval.
    pub fn new(symbol: &str, throttle: Duration) -> Self {
        Self {
            symbol: symbol.to_string(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_emit: None,
            throttle,
        }
    }

    /// Apply a [`BookEvent`] to the book.
    ///
    /// Volume of `0.0` removes the level at that price.
    pub fn on_event(&mut self, event: &BookEvent) {
        if event.is_buy {
            if event.volume == 0.0 {
                self.bids.remove(&Reverse(PriceKey(event.price)));
            } else {
                self.bids
                    .insert(Reverse(PriceKey(event.price)), event.volume);
            }
        } else {
            if event.volume == 0.0 {
                self.asks.remove(&PriceKey(event.price));
            } else {
                self.asks.insert(PriceKey(event.price), event.volume);
            }
        }
    }

    /// Build a [`BookSnapshot`] from the current state.
    pub fn snapshot(&self, time_msc: i64) -> BookSnapshot {
        BookSnapshot {
            symbol: self.symbol.clone(),
            time_msc,
            bids: self
                .bids
                .iter()
                .map(|(Reverse(PriceKey(price)), volume)| BookLevel {
                    price: *price,
                    volume: *volume,
                })
                .collect(),
            asks: self
                .asks
                .iter()
                .map(|(PriceKey(price), volume)| BookLevel {
                    price: *price,
                    volume: *volume,
                })
                .collect(),
        }
    }

    /// Emit a snapshot if the throttle interval has elapsed since the last
    /// emit, returning `None` otherwise.
    ///
    /// The first call always emits.
    pub fn maybe_emit(&mut self, time_msc: i64) -> Option<BookSnapshot> {
        let now = Instant::now();
        let should_emit = self
            .last_emit
            .map_or(true, |last| now.duration_since(last) >= self.throttle);
        if should_emit {
            self.last_emit = Some(now);
            Some(self.snapshot(time_msc))
        } else {
            None
        }
    }

    /// Clear all levels from the book. Call this when the book is reset
    /// (for example, on a snapshot reset from the EA).
    pub fn reset(&mut self) {
        self.bids.clear();
        self.asks.clear();
    }

    /// Number of bid levels.
    pub fn bid_levels(&self) -> usize {
        self.bids.len()
    }

    /// Number of ask levels.
    pub fn ask_levels(&self) -> usize {
        self.asks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_event(symbol: &str, is_buy: bool, price: f64, volume: f64) -> BookEvent {
        BookEvent {
            symbol: symbol.to_string(),
            time_msc: 1_710_000_500_000,
            is_buy,
            price,
            volume,
            event_seq: 0,
        }
    }

    #[test]
    fn add_update_and_remove_levels() {
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(100));

        // Add bid levels
        stream.on_event(&make_event("EURUSD", true, 1.08500, 10.0));
        stream.on_event(&make_event("EURUSD", true, 1.08490, 20.0));
        stream.on_event(&make_event("EURUSD", false, 1.08510, 15.0));
        stream.on_event(&make_event("EURUSD", false, 1.08520, 25.0));

        let snap = stream.snapshot(1_710_000_500_000);
        assert_eq!(snap.bids.len(), 2);
        assert_eq!(snap.asks.len(), 2);
        // Bids sorted highest first
        assert_eq!(snap.bids[0].price, 1.08500);
        assert_eq!(snap.bids[1].price, 1.08490);
        // Asks sorted lowest first
        assert_eq!(snap.asks[0].price, 1.08510);
        assert_eq!(snap.asks[1].price, 1.08520);

        // Update an existing level
        stream.on_event(&make_event("EURUSD", true, 1.08500, 99.0));
        let snap = stream.snapshot(1_710_000_500_000);
        assert_eq!(snap.bids[0].volume, 99.0);

        // Remove a level (volume 0)
        stream.on_event(&make_event("EURUSD", true, 1.08490, 0.0));
        let snap = stream.snapshot(1_710_000_500_000);
        assert_eq!(snap.bids.len(), 1);
        assert_eq!(snap.bids[0].price, 1.08500);
    }

    #[test]
    fn imbalance_math() {
        // No levels -> 0.0
        let stream = BookStream::new("EURUSD", Duration::from_millis(100));
        assert_eq!(stream.snapshot(0).imbalance(5), 0.0);

        // Only bids -> +1.0
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(100));
        stream.on_event(&make_event("EURUSD", true, 1.08500, 100.0));
        assert!((stream.snapshot(0).imbalance(5) - 1.0).abs() < f64::EPSILON);

        // Only asks -> -1.0
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(100));
        stream.on_event(&make_event("EURUSD", false, 1.08510, 100.0));
        assert!((stream.snapshot(0).imbalance(5) - (-1.0)).abs() < f64::EPSILON);

        // Balanced -> 0.0
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(100));
        stream.on_event(&make_event("EURUSD", true, 1.08500, 100.0));
        stream.on_event(&make_event("EURUSD", false, 1.08510, 100.0));
        assert!((stream.snapshot(0).imbalance(5) - 0.0).abs() < f64::EPSILON);

        // Top-N limiting: only first 2 levels counted
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(100));
        stream.on_event(&make_event("EURUSD", true, 1.08500, 10.0));
        stream.on_event(&make_event("EURUSD", true, 1.08490, 10.0));
        stream.on_event(&make_event("EURUSD", true, 1.08480, 100.0)); // beyond N=2
        stream.on_event(&make_event("EURUSD", false, 1.08510, 10.0));
        stream.on_event(&make_event("EURUSD", false, 1.08520, 10.0));
        let snap = stream.snapshot(0);
        assert!((snap.imbalance(2) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn weighted_mid() {
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(100));

        // Empty -> 0.0
        let snap = stream.snapshot(0);
        assert_eq!(snap.weighted_mid(), 0.0);

        // One side only -> 0.0
        stream.on_event(&make_event("EURUSD", true, 1.08500, 100.0));
        let snap = stream.snapshot(0);
        assert_eq!(snap.weighted_mid(), 0.0);

        // Both sides: bid 1.08500 @ 100, ask 1.08510 @ 200
        // weighted_mid = (1.08500 * 200 + 1.08510 * 100) / 300
        stream.on_event(&make_event("EURUSD", false, 1.08510, 200.0));
        let snap = stream.snapshot(0);
        let expected = (1.08500 * 200.0 + 1.08510 * 100.0) / 300.0;
        assert!((snap.weighted_mid() - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn throttle_first_call_always_emits() {
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(50));

        assert!(stream.maybe_emit(1_710_000_500_000).is_some());
        // Immediate second call should be throttled
        assert!(stream.maybe_emit(1_710_000_500_000).is_none());
    }

    #[test]
    fn reset_clears_all_levels() {
        let mut stream = BookStream::new("EURUSD", Duration::from_millis(100));

        stream.on_event(&make_event("EURUSD", true, 1.08500, 10.0));
        stream.on_event(&make_event("EURUSD", false, 1.08510, 10.0));
        assert_eq!(stream.bid_levels(), 1);
        assert_eq!(stream.ask_levels(), 1);

        stream.reset();
        assert_eq!(stream.bid_levels(), 0);
        assert_eq!(stream.ask_levels(), 0);

        let snap = stream.snapshot(0);
        assert!(snap.bids.is_empty());
        assert!(snap.asks.is_empty());
    }
}

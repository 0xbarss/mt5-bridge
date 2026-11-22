//! Tick-to-bar aggregation and a bounded bar cache.
//!
//! The [`BarAggregator`] consumes a stream of [`Tick`]s and emits completed
//! [`Bar`]s for time-based, tick-count, volume, and range bar kinds. The
//! [`BarCache`] caches recently fetched bars keyed by `(symbol, timeframe)`
//! with a bounded length and a TTL so repeated `copy_bars` calls can be served
//! from memory.

use crate::types::{Bar, Tick, Timeframe};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Kind of bar to aggregate ticks into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BarKind {
    /// Standard time-based bars (M1, M5, H1, …). Ticks are bucketed by
    /// `tick.time_msc / interval_ms`.
    Time(Timeframe),
    /// Emit a bar every `n` ticks regardless of time or volume.
    TickCount(usize),
    /// Emit a bar once accumulated tick volume reaches `threshold`.
    Volume(f64),
    /// Emit a bar once the high-low range reaches `threshold` price units.
    Range(f64),
}

impl BarKind {
    /// Interval in milliseconds for [`BarKind::Time`]; `None` for non-time kinds.
    pub fn interval_ms(self) -> Option<i64> {
        match self {
            BarKind::Time(tf) => Some(tf.seconds() * 1000),
            _ => None,
        }
    }
}

/// Aggregates [`Tick`]s into [`Bar`]s.
///
/// Each call to [`on_tick`](Self::on_tick) folds one tick into the current
/// in-progress bar. When the bar is complete (time bucket rolled over, tick
/// count reached, volume threshold met, or range threshold met) the completed
/// bar is returned and a new bar is started.
///
/// The price used for OHLC is `tick.last` when it is positive, otherwise the
/// mid-price `(bid + ask) / 2`. Volume is taken from `tick.volume`.
pub struct BarAggregator {
    symbol: String,
    kind: BarKind,
    current: Option<Bar>,
    bucket_key: Option<i64>,
    tick_count: usize,
    volume_sum: f64,
}

impl BarAggregator {
    /// Create a new aggregator for `symbol` producing bars of `kind`.
    pub fn new(symbol: impl Into<String>, kind: BarKind) -> Self {
        Self {
            symbol: symbol.into(),
            kind,
            current: None,
            bucket_key: None,
            tick_count: 0,
            volume_sum: 0.0,
        }
    }

    /// Symbol this aggregator tracks.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Bar kind this aggregator produces.
    pub fn kind(&self) -> &BarKind {
        &self.kind
    }

    /// Reference to the in-progress (not yet emitted) bar, if any.
    pub fn current(&self) -> Option<&Bar> {
        self.current.as_ref()
    }

    /// Number of ticks folded into the current bar.
    pub fn tick_count(&self) -> usize {
        self.tick_count
    }

    /// Accumulated volume of the current bar.
    pub fn volume_sum(&self) -> f64 {
        self.volume_sum
    }

    /// Fold one tick into the current bar.
    ///
    /// Returns `Some(bar)` when the current bar is complete and a new one has
    /// been started. Returns `None` while the current bar is still forming.
    pub fn on_tick(&mut self, tick: &Tick) -> Option<Bar> {
        let price = tick_price(tick);
        let volume = tick.volume as f64;

        match self.kind {
            BarKind::Time(tf) => self.on_tick_time(tick, price, volume, tf),
            BarKind::TickCount(n) => self.on_tick_count(price, volume, n),
            BarKind::Volume(threshold) => self.on_tick_volume(price, volume, threshold),
            BarKind::Range(threshold) => self.on_tick_range(price, volume, threshold),
        }
    }

    /// Flush the in-progress bar and return it, resetting the aggregator.
    ///
    /// Useful at end of stream or session to emit the final incomplete bar.
    pub fn finalize(&mut self) -> Option<Bar> {
        let bar = self.current.take();
        self.bucket_key = None;
        self.tick_count = 0;
        self.volume_sum = 0.0;
        bar
    }

    fn on_tick_time(&mut self, tick: &Tick, price: f64, volume: f64, tf: Timeframe) -> Option<Bar> {
        let interval_ms = tf.seconds() * 1000;
        let bucket_key = tick.time_msc / interval_ms;
        let bucket_start_sec = (bucket_key * interval_ms) / 1000;

        let new_bucket = self.bucket_key != Some(bucket_key);
        let completed = if new_bucket {
            self.current.take()
        } else {
            None
        };

        if let Some(ref mut bar) = self.current {
            bar.high = bar.high.max(price);
            bar.low = bar.low.min(price);
            bar.close = price;
            bar.volume += volume;
        } else {
            self.current = Some(Bar::new(
                bucket_start_sec,
                price,
                price,
                price,
                price,
                volume,
            ));
        }
        self.bucket_key = Some(bucket_key);
        self.tick_count += 1;
        self.volume_sum += volume;

        completed
    }

    fn on_tick_count(&mut self, price: f64, volume: f64, n: usize) -> Option<Bar> {
        if self.current.is_none() {
            self.current = Some(Bar::new(0, price, price, price, price, 0.0));
        }
        if let Some(ref mut bar) = self.current {
            bar.high = bar.high.max(price);
            bar.low = bar.low.min(price);
            bar.close = price;
            bar.volume += volume;
        }
        self.tick_count += 1;
        self.volume_sum += volume;

        if self.tick_count >= n {
            return self.current.take();
        }
        None
    }

    fn on_tick_volume(&mut self, price: f64, volume: f64, threshold: f64) -> Option<Bar> {
        if self.current.is_none() {
            self.current = Some(Bar::new(0, price, price, price, price, 0.0));
        }
        if let Some(ref mut bar) = self.current {
            bar.high = bar.high.max(price);
            bar.low = bar.low.min(price);
            bar.close = price;
            bar.volume += volume;
        }
        self.tick_count += 1;
        self.volume_sum += volume;

        if self.volume_sum >= threshold {
            return self.current.take();
        }
        None
    }

    fn on_tick_range(&mut self, price: f64, volume: f64, threshold: f64) -> Option<Bar> {
        if self.current.is_none() {
            self.current = Some(Bar::new(0, price, price, price, price, 0.0));
        }
        if let Some(ref mut bar) = self.current {
            bar.high = bar.high.max(price);
            bar.low = bar.low.min(price);
            bar.close = price;
            bar.volume += volume;
        }
        self.tick_count += 1;
        self.volume_sum += volume;

        if let Some(bar) = self.current {
            if bar.high - bar.low >= threshold {
                return self.current.take();
            }
        }
        None
    }
}

/// Price used for OHLC from a tick.
fn tick_price(tick: &Tick) -> f64 {
    if tick.last > 0.0 {
        tick.last
    } else {
        (tick.bid + tick.ask) / 2.0
    }
}

/// A single cached entry: a bounded deque of bars plus the last update instant.
#[derive(Debug, Clone)]
struct BarCacheEntry {
    bars: VecDeque<Bar>,
    updated_at: Instant,
}

/// Bounded, TTL-backed cache of bars keyed by `(symbol, timeframe)`.
///
/// Serves repeated `copy_bars` calls from memory. Each entry holds at most
/// `max_bars` bars (oldest dropped on overflow) and is evicted entirely once
/// older than `ttl`. Use [`invalidate`](Self::invalidate) when a new bar
/// arrives for a key so consumers do not serve stale data.
pub struct BarCache {
    map: HashMap<(String, Timeframe), BarCacheEntry>,
    max_bars: usize,
    ttl: Duration,
}

impl BarCache {
    /// Create a cache holding at most `max_bars` bars per key, evicting entries
    /// untouched for longer than `ttl`.
    pub fn new(max_bars: usize, ttl: Duration) -> Self {
        Self {
            map: HashMap::new(),
            max_bars,
            ttl,
        }
    }

    /// Number of keys currently cached.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True if no keys are cached.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Return the cached bars for `symbol`/`timeframe`, if present and not expired.
    pub fn get(&self, symbol: &str, timeframe: Timeframe) -> Option<&VecDeque<Bar>> {
        let entry = self.map.get(&(symbol.to_string(), timeframe))?;
        if entry.updated_at.elapsed() > self.ttl {
            return None;
        }
        Some(&entry.bars)
    }

    /// Append `bar` to the cache for `symbol`/`timeframe`, dropping the oldest
    /// bar once the per-key bound is exceeded. Resets the entry TTL.
    pub fn push(&mut self, symbol: &str, timeframe: Timeframe, bar: Bar) {
        let key = (symbol.to_string(), timeframe);
        let entry = self.map.entry(key).or_insert_with(|| BarCacheEntry {
            bars: VecDeque::new(),
            updated_at: Instant::now(),
        });
        if entry.bars.len() >= self.max_bars {
            entry.bars.pop_front();
        }
        entry.bars.push_back(bar);
        entry.updated_at = Instant::now();
    }

    /// Drop the cached entry for `symbol`/`timeframe`, if any. Call this when a
    /// new bar arrives so consumers do not serve stale data.
    pub fn invalidate(&mut self, symbol: &str, timeframe: Timeframe) {
        self.map.remove(&(symbol.to_string(), timeframe));
    }

    /// Remove every entry whose last update is older than `ttl`, returning the
    /// number of entries evicted. Call periodically for maintenance.
    pub fn evict_expired(&mut self) -> usize {
        let before = self.map.len();
        self.map
            .retain(|_, entry| entry.updated_at.elapsed() <= self.ttl);
        before - self.map.len()
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

impl Default for BarCache {
    fn default() -> Self {
        Self::new(500, Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(time_msc: i64, bid: f64, ask: f64, last: f64, volume: u64) -> Tick {
        Tick {
            symbol: "EURUSD".into(),
            time_msc,
            time: time_msc / 1000,
            bid,
            ask,
            last,
            volume,
            flags: 0,
            event_seq: 0,
        }
    }

    #[test]
    fn time_bars_bucket_by_interval() {
        // M1 = 60_000 ms. Two ticks in the same minute -> one bar; a tick in
        // the next minute closes the first and opens the second.
        let mut agg = BarAggregator::new("EURUSD", BarKind::Time(Timeframe::M1));

        let t0 = 1_700_000_000_000i64; // 20s into a minute
        let interval_ms = 60_000;
        let bucket_start_sec = (t0 / interval_ms) * interval_ms / 1000;
        assert!(agg.on_tick(&tick(t0, 1.0, 1.001, 1.0005, 10)).is_none());
        assert!(agg
            .on_tick(&tick(t0 + 10_000, 1.002, 1.003, 1.0025, 20))
            .is_none());

        // Next minute: first bar should be emitted.
        let t1 = t0 + 60_000;
        let bar = agg
            .on_tick(&tick(t1, 1.005, 1.006, 1.0055, 30))
            .expect("bar expected");
        assert_eq!(bar.time, bucket_start_sec);
        assert_eq!(bar.open, 1.0005);
        assert_eq!(bar.high, 1.0025);
        assert_eq!(bar.low, 1.0005);
        assert_eq!(bar.close, 1.0025);
        assert_eq!(bar.volume, 30.0);

        // New bar started for the second minute.
        let cur = agg.current().expect("current bar expected");
        assert_eq!(cur.time, bucket_start_sec + 60);
        assert_eq!(cur.open, 1.0055);
    }

    #[test]
    fn time_bars_across_hour_boundary() {
        let mut agg = BarAggregator::new("EURUSD", BarKind::Time(Timeframe::H1));
        // 3_600_000 ms per hour. h0 is 20min into an hour.
        let h0 = 1_700_000_000_000i64;
        let interval_ms = 3_600_000;
        let bucket_start_sec = (h0 / interval_ms) * interval_ms / 1000;
        assert!(agg.on_tick(&tick(h0, 1.0, 1.001, 1.0005, 5)).is_none());
        let h1 = h0 + 3_600_000;
        let bar = agg
            .on_tick(&tick(h1, 1.01, 1.011, 1.0105, 7))
            .expect("bar expected");
        assert_eq!(bar.time, bucket_start_sec);
        assert_eq!(bar.close, 1.0005);
        assert_eq!(bar.volume, 5.0);
    }

    #[test]
    fn tick_count_bars_emit_after_n_ticks() {
        let mut agg = BarAggregator::new("EURUSD", BarKind::TickCount(3));
        assert!(agg.on_tick(&tick(1, 1.0, 1.001, 1.0005, 1)).is_none());
        assert!(agg.on_tick(&tick(2, 1.002, 1.003, 1.0025, 2)).is_none());
        let bar = agg
            .on_tick(&tick(3, 1.004, 1.005, 1.0045, 3))
            .expect("bar expected");
        assert_eq!(bar.open, 1.0005);
        assert_eq!(bar.high, 1.0045);
        assert_eq!(bar.low, 1.0005);
        assert_eq!(bar.close, 1.0045);
        assert_eq!(bar.volume, 6.0);
    }

    #[test]
    fn volume_bars_emit_at_threshold() {
        let mut agg = BarAggregator::new("EURUSD", BarKind::Volume(10.0));
        assert!(agg.on_tick(&tick(1, 1.0, 1.001, 1.0005, 4)).is_none()); // sum 4
        assert!(agg.on_tick(&tick(2, 1.002, 1.003, 1.0025, 3)).is_none()); // sum 7
        let bar = agg
            .on_tick(&tick(3, 1.004, 1.005, 1.0045, 5))
            .expect("bar expected"); // sum 12 >= 10
        assert_eq!(bar.volume, 12.0);
        assert_eq!(bar.close, 1.0045);
    }

    #[test]
    fn range_bars_emit_at_threshold() {
        let mut agg = BarAggregator::new("EURUSD", BarKind::Range(0.005));
        assert!(agg.on_tick(&tick(1, 1.0, 1.001, 1.0005, 1)).is_none()); // range = 0
        assert!(agg.on_tick(&tick(2, 1.001, 1.002, 1.0015, 2)).is_none()); // range = 0.001 < 0.005
        let bar = agg
            .on_tick(&tick(3, 1.006, 1.007, 1.0065, 3))
            .expect("bar expected"); // range = 0.006 >= 0.005
        assert!((bar.high - bar.low) >= 0.005);
        assert_eq!(bar.volume, 6.0);
        assert_eq!(bar.close, 1.0065);
    }

    #[test]
    fn zero_volume_ticks_do_not_panic() {
        let mut agg = BarAggregator::new("EURUSD", BarKind::Volume(5.0));
        assert!(agg.on_tick(&tick(1, 1.0, 1.001, 1.0005, 0)).is_none());
        assert!(agg.on_tick(&tick(2, 1.002, 1.003, 1.0025, 0)).is_none());
        // volume stays 0, threshold never reached
        assert!(agg.current().is_some());
    }

    #[test]
    fn last_price_falls_back_to_mid() {
        let mut agg = BarAggregator::new("EURUSD", BarKind::TickCount(1));
        // last == 0 -> use mid (1.0 + 1.002) / 2 = 1.001
        let bar = agg
            .on_tick(&tick(1, 1.0, 1.002, 0.0, 1))
            .expect("bar expected");
        assert!((bar.close - 1.001).abs() < 1e-9);
    }

    #[test]
    fn finalize_flushes_incomplete_bar() {
        let mut agg = BarAggregator::new("EURUSD", BarKind::TickCount(10));
        assert!(agg.on_tick(&tick(1, 1.0, 1.001, 1.0005, 1)).is_none());
        let bar = agg.finalize().expect("finalized bar expected");
        assert_eq!(bar.close, 1.0005);
        assert!(agg.current().is_none());
    }

    #[test]
    fn cache_get_and_push_respects_bound() {
        let mut cache = BarCache::new(2, Duration::from_secs(60));
        cache.push(
            "EURUSD",
            Timeframe::M1,
            Bar::new(1, 1.0, 1.1, 0.9, 1.05, 10.0),
        );
        cache.push(
            "EURUSD",
            Timeframe::M1,
            Bar::new(2, 1.05, 1.15, 1.0, 1.1, 10.0),
        );
        cache.push(
            "EURUSD",
            Timeframe::M1,
            Bar::new(3, 1.1, 1.2, 1.05, 1.15, 10.0),
        ); // evicts first

        let bars = cache.get("EURUSD", Timeframe::M1).expect("cached");
        assert_eq!(bars.len(), 2);
        assert_eq!(bars.front().unwrap().time, 2);
        assert_eq!(bars.back().unwrap().time, 3);
    }

    #[test]
    fn cache_evicts_expired_entries() {
        let mut cache = BarCache::new(10, Duration::from_nanos(1));
        cache.push(
            "EURUSD",
            Timeframe::M1,
            Bar::new(1, 1.0, 1.1, 0.9, 1.05, 10.0),
        );
        // Force expiry by sleeping briefly.
        std::thread::sleep(Duration::from_nanos(10));
        let evicted = cache.evict_expired();
        assert_eq!(evicted, 1);
        assert!(cache.get("EURUSD", Timeframe::M1).is_none());
    }

    #[test]
    fn cache_invalidate_removes_entry() {
        let mut cache = BarCache::new(10, Duration::from_secs(60));
        cache.push(
            "EURUSD",
            Timeframe::M1,
            Bar::new(1, 1.0, 1.1, 0.9, 1.05, 10.0),
        );
        cache.invalidate("EURUSD", Timeframe::M1);
        assert!(cache.get("EURUSD", Timeframe::M1).is_none());
    }

    #[test]
    fn cache_different_keys_are_independent() {
        let mut cache = BarCache::new(10, Duration::from_secs(60));
        cache.push(
            "EURUSD",
            Timeframe::M1,
            Bar::new(1, 1.0, 1.1, 0.9, 1.05, 10.0),
        );
        cache.push(
            "GBPUSD",
            Timeframe::M1,
            Bar::new(2, 1.0, 1.1, 0.9, 1.05, 10.0),
        );
        assert!(cache.get("EURUSD", Timeframe::M1).is_some());
        assert!(cache.get("GBPUSD", Timeframe::M1).is_some());
        cache.invalidate("EURUSD", Timeframe::M1);
        assert!(cache.get("EURUSD", Timeframe::M1).is_none());
        assert!(cache.get("GBPUSD", Timeframe::M1).is_some());
    }

    #[test]
    fn default_cache_has_sensible_defaults() {
        let cache = BarCache::default();
        assert_eq!(cache.max_bars_field_for_test(), 500);
        assert_eq!(cache.ttl_field_for_test(), Duration::from_secs(60));
    }

    // Test-only accessors to avoid dead-code on private fields.
    impl BarCache {
        fn max_bars_field_for_test(&self) -> usize {
            self.max_bars
        }
        fn ttl_field_for_test(&self) -> Duration {
            self.ttl
        }
    }
}

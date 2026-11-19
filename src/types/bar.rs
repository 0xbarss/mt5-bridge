use crate::ffi::Mt5BarEvent;
use crate::types::rate::Rate;
use serde::{Deserialize, Serialize};

/// Clean OHLCV bar representation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Bar {
    pub fn new(time: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Self {
            time,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    /// Construct from an incoming low-level bar event.
    pub fn from_event(raw: &Mt5BarEvent) -> (String, i32, Self) {
        let sym_len = raw
            .symbol
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw.symbol.len());
        let symbol = String::from_utf8_lossy(&raw.symbol[..sym_len])
            .trim()
            .to_string();
        let bar = Self {
            time: raw.time,
            open: raw.open,
            high: raw.high,
            low: raw.low,
            close: raw.close,
            volume: raw.volume as f64,
        };
        (symbol, raw.timeframe, bar)
    }

    /// Mid-point price: `(high + low) / 2.0`.
    pub fn mid(&self) -> f64 {
        (self.high + self.low) / 2.0
    }

    /// Typical price: `(high + low + close) / 3.0`.
    pub fn typical_price(&self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }

    /// High - Low range.
    pub fn range(&self) -> f64 {
        self.high - self.low
    }

    /// Absolute candle body size: `|close - open|`.
    pub fn body(&self) -> f64 {
        (self.close - self.open).abs()
    }

    /// True range compared to the previous bar's close.
    pub fn true_range(&self, prev_close: f64) -> f64 {
        let hl = self.high - self.low;
        let hc = (self.high - prev_close).abs();
        let lc = (self.low - prev_close).abs();
        hl.max(hc).max(lc)
    }

    pub fn is_bullish(&self) -> bool {
        self.close > self.open
    }

    pub fn is_bearish(&self) -> bool {
        self.close < self.open
    }
}

impl From<Rate> for Bar {
    fn from(r: Rate) -> Self {
        Self {
            time: r.time,
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
            volume: r.volume as f64,
        }
    }
}

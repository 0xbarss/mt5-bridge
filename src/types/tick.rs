use crate::ffi::{Mt5Tick, Mt5TickEvent};
pub use crate::types::trade_event::BookEvent;
use serde::{Deserialize, Serialize};

/// Real-time quote tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    pub symbol: String,
    /// Time of the last quote in milliseconds (UTC unix timestamp).
    pub time_msc: i64,
    /// Time of the last quote in seconds (UTC unix timestamp).
    pub time: i64,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume: u64,
    pub flags: u32,
    #[serde(default)]
    pub event_seq: u64,
}

impl Tick {
    pub(crate) fn from_raw(symbol: &str, raw: Mt5Tick) -> Self {
        Self {
            symbol: symbol.to_string(),
            time_msc: raw.time,
            time: raw.time / 1000,
            bid: raw.bid,
            ask: raw.ask,
            last: raw.last,
            volume: raw.volume,
            flags: raw.flags,
            event_seq: 0,
        }
    }

    pub fn from_event(raw: Mt5TickEvent) -> Self {
        let sym_len = raw
            .symbol
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw.symbol.len());
        let symbol = String::from_utf8_lossy(&raw.symbol[..sym_len])
            .trim()
            .to_string();
        Self {
            symbol,
            time_msc: raw.time_msc,
            time: raw.time_msc / 1000,
            bid: raw.bid,
            ask: raw.ask,
            last: raw.last,
            volume: raw.volume,
            flags: raw.flags,
            event_seq: raw.event_seq,
        }
    }

    /// Spread in quote currency (`ask - bid`).
    pub fn spread(&self) -> f64 {
        self.ask - self.bid
    }

    /// Mid-price (`(ask + bid) / 2.0`).
    pub fn mid(&self) -> f64 {
        (self.ask + self.bid) / 2.0
    }
}

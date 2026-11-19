use crate::ffi::Mt5Rate;
use serde::{Deserialize, Serialize};

/// Raw rate structure returned by `CopyRates`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Rate {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub spread: i32,
    pub real_volume: i64,
}

impl From<Mt5Rate> for Rate {
    fn from(r: Mt5Rate) -> Self {
        Self {
            time: r.time,
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
            volume: r.volume,
            spread: r.spread,
            real_volume: r._rv,
        }
    }
}

use crate::ffi::Mt5Position;
use crate::types::order_enums::OrderType;
use serde::{Deserialize, Serialize};

/// Detailed information for an active open position in MetaTrader 5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub ticket: u64,
    pub time: i64,
    pub position_type: OrderType,
    pub magic: u64,
    pub volume: f64,
    pub price_open: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub price_current: f64,
    pub profit: f64,
    pub swap: f64,
    pub symbol: String,
    pub comment: String,
}

impl Position {
    pub fn from_raw(raw: Mt5Position) -> Self {
        let sym_len = raw
            .symbol
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(raw.symbol.len());
        let sym = String::from_utf8_lossy(&raw.symbol[..sym_len])
            .trim()
            .to_string();

        let cmt_len = raw
            .comment
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(raw.comment.len());
        let cmt = String::from_utf8_lossy(&raw.comment[..cmt_len])
            .trim()
            .to_string();

        let ptype = if raw.position_type == 1 {
            OrderType::Sell
        } else {
            OrderType::Buy
        };

        Self {
            ticket: raw.ticket,
            time: raw.time,
            position_type: ptype,
            magic: raw.magic,
            volume: raw.volume,
            price_open: raw.price_open,
            stop_loss: raw.sl,
            take_profit: raw.tp,
            price_current: raw.price_current,
            profit: raw.profit,
            swap: raw.swap,
            symbol: sym,
            comment: cmt,
        }
    }

    pub fn is_buy(&self) -> bool {
        self.position_type.is_buy()
    }

    pub fn is_sell(&self) -> bool {
        self.position_type.is_sell()
    }

    pub fn matches_magic(&self, magic: u64) -> bool {
        self.magic == magic
    }
}

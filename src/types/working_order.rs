use crate::ffi::Mt5Order;
use crate::types::order_enums::OrderType;
use serde::{Deserialize, Serialize};

/// Detailed information for an active working pending order in MetaTrader 5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkingOrder {
    pub ticket: u64,
    pub time_setup: i64,
    pub order_type: OrderType,
    pub magic: u64,
    pub volume_initial: f64,
    pub volume_current: f64,
    pub price_open: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub price_current: f64,
    pub symbol: String,
    pub comment: String,
}

impl WorkingOrder {
    pub fn from_raw(raw: Mt5Order) -> Self {
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

        let otype = match raw.order_type {
            2 => OrderType::BuyLimit,
            3 => OrderType::SellLimit,
            4 => OrderType::BuyStop,
            5 => OrderType::SellStop,
            1 => OrderType::Sell,
            _ => OrderType::Buy,
        };

        Self {
            ticket: raw.ticket,
            time_setup: raw.time_setup,
            order_type: otype,
            magic: raw.magic,
            volume_initial: raw.volume_initial,
            volume_current: raw.volume_current,
            price_open: raw.price_open,
            stop_loss: raw.sl,
            take_profit: raw.tp,
            price_current: raw.price_current,
            symbol: sym,
            comment: cmt,
        }
    }

    pub fn matches_magic(&self, magic: u64) -> bool {
        self.magic == magic
    }
}

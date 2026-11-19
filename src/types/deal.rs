use crate::ffi::Mt5Deal;
use crate::types::order_enums::OrderType;
use serde::{Deserialize, Serialize};

/// Direction of a completed deal on the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DealEntry {
    /// Opens or increases a position.
    In,
    /// Closes or reduces a position.
    Out,
    /// Reverses a position.
    InOut,
    /// Closed by an opposite position.
    OutBy,
    /// Any other/unrecognised entry code.
    Other,
}

impl DealEntry {
    /// `true` if this deal opened or increased exposure (the fills that satisfy an order).
    pub fn is_entry(self) -> bool {
        matches!(self, DealEntry::In | DealEntry::InOut)
    }

    /// `true` if this deal closed or reduced exposure.
    pub fn is_exit(self) -> bool {
        matches!(self, DealEntry::Out | DealEntry::InOut | DealEntry::OutBy)
    }

    pub fn from_raw(v: i32) -> Self {
        match v {
            0 => DealEntry::In,
            1 => DealEntry::Out,
            2 => DealEntry::InOut,
            3 => DealEntry::OutBy,
            _ => DealEntry::Other,
        }
    }
}

/// A completed broker-side deal from MT5 trade history.
///
/// Deal history is the authoritative record of what actually executed, independent of
/// whether the synchronous [`TradeResult`](crate::types::trade_result::TradeResult) ever made it back over the pipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deal {
    pub ticket: u64,
    /// Ticket of the order that produced this deal.
    pub order: u64,
    /// Position this deal belongs to (`DEAL_POSITION_ID`).
    pub position_id: u64,
    /// Execution time (UTC seconds unless the EA is configured for raw broker time).
    pub time: i64,
    pub direction: OrderType,
    pub entry: DealEntry,
    pub magic: u64,
    pub volume: f64,
    pub price: f64,
    pub commission: f64,
    pub swap: f64,
    pub profit: f64,
    pub symbol: String,
    pub comment: String,
}

impl Deal {
    pub fn from_raw(raw: Mt5Deal) -> Self {
        let fixed = |bytes: &[u8; 32]| {
            let len = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..len]).trim().to_string()
        };
        // Copy packed fields to locals (avoids taking references to packed fields).
        let (sym, cmt) = (raw.symbol, raw.comment);
        Self {
            ticket: raw.ticket,
            order: raw.order,
            position_id: raw.position_id,
            time: raw.time,
            direction: if raw.deal_type == 1 {
                OrderType::Sell
            } else {
                OrderType::Buy
            },
            entry: DealEntry::from_raw(raw.entry),
            magic: raw.magic,
            volume: raw.volume,
            price: raw.price,
            commission: raw.commission,
            swap: raw.swap,
            profit: raw.profit,
            symbol: fixed(&sym),
            comment: fixed(&cmt),
        }
    }
}

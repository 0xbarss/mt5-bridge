use crate::ffi::{Mt5BookEvent, Mt5TradeEvent};
use crate::types::order_enums::OrderType;
use serde::{Deserialize, Serialize};

/// Asynchronous trade transaction event from MT5 (protocol v6+).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeEvent {
    pub deal: u64,
    pub order: u64,
    pub position: u64,
    pub time: i64,
    pub trans_type: i32,
    pub order_type: OrderType,
    pub price: f64,
    pub volume: f64,
    pub sl: f64,
    pub tp: f64,
    pub symbol: String,
    pub comment: String,
    #[serde(default)]
    pub deal_entry: i32,
    #[serde(default)]
    pub deal_type: i32,
    #[serde(default)]
    pub deal_magic: u64,
    #[serde(default)]
    pub deal_position_id: u64,
    #[serde(default)]
    pub deal_reason: i32,
    #[serde(default)]
    pub event_seq: u64,
}

impl TradeEvent {
    pub fn from_raw(raw: Mt5TradeEvent) -> Self {
        let sym_len = raw
            .symbol
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw.symbol.len());
        let symbol = String::from_utf8_lossy(&raw.symbol[..sym_len])
            .trim()
            .to_string();
        let cmt_len = raw
            .comment
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw.comment.len());
        let comment = String::from_utf8_lossy(&raw.comment[..cmt_len])
            .trim()
            .to_string();
        let order_type = match raw.order_type {
            0 => OrderType::Buy,
            1 => OrderType::Sell,
            2 => OrderType::BuyLimit,
            3 => OrderType::SellLimit,
            4 => OrderType::BuyStop,
            5 => OrderType::SellStop,
            _ => OrderType::Buy,
        };
        Self {
            deal: raw.deal,
            order: raw.order,
            position: raw.position,
            time: raw.time,
            trans_type: raw.trans_type,
            order_type,
            price: raw.price,
            volume: raw.volume,
            sl: raw.sl,
            tp: raw.tp,
            symbol,
            comment,
            deal_entry: raw.deal_entry,
            deal_type: raw.deal_type,
            deal_magic: raw.deal_magic,
            deal_position_id: raw.deal_position_id,
            deal_reason: raw.deal_reason,
            event_seq: raw.event_seq,
        }
    }
}

/// Asynchronous depth-of-market book event from MT5 (protocol v5+).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookEvent {
    pub symbol: String,
    pub time_msc: i64,
    pub is_buy: bool,
    pub price: f64,
    pub volume: f64,
    #[serde(default)]
    pub event_seq: u64,
}

impl BookEvent {
    pub fn from_raw(raw: Mt5BookEvent) -> Self {
        let time_msc = raw.time_msc;
        if time_msc > 0 && time_msc < 1_000_000_000_000 {
            tracing::warn!(time_msc, "book event timestamp is not epoch milliseconds");
        }
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
            time_msc,
            is_buy: raw.book_type == 1,
            price: raw.price,
            volume: raw.volume,
            event_seq: raw.event_seq,
        }
    }
}

/// Individual deal execution fill attributed to an order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub deal: u64,
    pub volume: f64,
    pub price: f64,
    pub time: i64,
    pub deal_type: i32,
    pub deal_entry: i32,
}

impl From<&TradeEvent> for Fill {
    fn from(ev: &TradeEvent) -> Self {
        Self {
            deal: ev.deal,
            volume: ev.volume,
            price: ev.price,
            time: ev.time,
            deal_type: ev.deal_type,
            deal_entry: ev.deal_entry,
        }
    }
}

/// Outcome effect when applying an asynchronous trade event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEffect {
    /// Fill or acceptance applied to order state.
    Applied,
    /// Event did not carry fill semantics or was irrelevant.
    Ignored,
    /// Exit deal was ignored to prevent double counting or reducing filled volume.
    ExitDealIgnored,
    /// Deal was already recorded (idempotent deduplication).
    Duplicate,
    /// Order was already in a terminal state (Filled, Cancelled, Rejected, Expired) and cannot be modified.
    TerminalStateIgnored,
}

/// Concrete evidence recorded when an order is determined to be Rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RejectionEvidence {
    /// Order was never transmitted to the network (e.g. write-ahead failed or local validation rejected).
    NeverTransmitted,
    /// Order was rejected by EA-side preflight risk checks before hitting the broker.
    PreflightRejected,
    /// Broker returned a definitive rejection retcode (e.g. 10019 NO_MONEY, or Binance -2019).
    /// Signed to preserve negative exchange codes.
    BrokerRetcode(i32),
    /// Multiple reconciliation passes observed the order absent with intact comments and no candidates.
    ReconciledAbsent { observations: u32, span_secs: i64 },
    /// An operator inspected the broker and resolved an ambiguous order as rejected.
    ///
    /// The note records the operator's justification.
    OperatorResolved { note: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_event_round_trip() {
        let mut sym = [0u8; 32];
        sym[..6].copy_from_slice(b"GBPUSD");
        let raw = Mt5BookEvent {
            symbol: sym,
            time_msc: 1_710_000_500_123,
            book_type: 2,
            _pad: 0,
            price: 1.27500,
            volume: 50.0,
            event_seq: 101,
        };
        let event = BookEvent::from_raw(raw);
        assert_eq!(event.symbol, "GBPUSD");
        assert_eq!(event.time_msc, 1_710_000_500_123);
        assert!(!event.is_buy);
        assert_eq!(event.price, 1.27500);
        assert_eq!(event.volume, 50.0);
        assert_eq!(event.event_seq, 101);
    }

    #[test]
    fn book_event_handles_uptime_timestamp_without_panic() {
        let mut sym = [0u8; 32];
        sym[..6].copy_from_slice(b"EURUSD");
        let raw = Mt5BookEvent {
            symbol: sym,
            time_msc: 123_456,
            book_type: 1,
            _pad: 0,
            price: 1.08500,
            volume: 10.0,
            event_seq: 1,
        };
        let event = BookEvent::from_raw(raw);
        assert_eq!(event.time_msc, 123_456);
        assert_eq!(event.event_seq, 1);
    }
}

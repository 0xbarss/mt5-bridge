pub mod account;
pub mod bar;
pub mod deal;
pub mod history;
pub mod level;
pub mod mismatch;
pub mod order_enums;
pub mod order_request;
pub mod position;
pub mod price_ticks;
pub mod rate;
pub mod stats;
pub mod stream_mode;
pub mod symbol;
pub mod tick;
pub mod timeframe;
pub mod tracked_order;
pub mod trade_event;
pub mod trade_result;
pub mod trade_rule;
pub mod units;
pub mod wire_id;
pub mod working_order;

#[cfg(test)]
mod tests;

pub use account::{AccountInfo, PingResult};
pub use bar::Bar;
pub use deal::{Deal, DealEntry};
pub use history::{HistoryResult, HistoryState};
pub use level::Level;
pub use mismatch::{approx_price_eq, approx_volume_eq, AttributeMismatch, MismatchKind};
pub use order_enums::{OrderState, OrderType, TradeStatus};
pub use order_request::{OrderRequest, OrderRequestBuilder};
pub use position::Position;
pub use price_ticks::{calculate_sl_ticks, calculate_tp_ticks, price_to_ticks, ticks_to_price};
pub use rate::Rate;
pub use stats::BridgeStats;
pub use stream_mode::StreamMode;
pub use symbol::SymbolInfo;
pub use tick::Tick;
pub use timeframe::Timeframe;
pub use tracked_order::TrackedOrder;
pub use trade_event::{BookEvent, EventEffect, Fill, RejectionEvidence, TradeEvent};
pub use trade_result::TradeResult;
pub use trade_rule::TradeRule;
pub use units::{Magic, Points, Price, Symbol, Ticket, Volume};
pub use wire_id::{
    comment_matches_client_order_id, parse_wire_id, truncate_utf8, wire_id,
    MAX_CLIENT_ORDER_ID_BYTES, MT5_COMMENT_MAX_BYTES, WIRE_COMMENT_PREFIX, WIRE_ID_LEN,
};
pub use working_order::WorkingOrder;

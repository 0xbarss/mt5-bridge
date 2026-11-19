use serde::{Deserialize, Serialize};

/// Order type matching MetaTrader 5 `ENUM_ORDER_TYPE`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Buy = 0,
    Sell = 1,
    BuyLimit = 2,
    SellLimit = 3,
    BuyStop = 4,
    SellStop = 5,
}

impl OrderType {
    pub fn is_buy(self) -> bool {
        matches!(
            self,
            OrderType::Buy | OrderType::BuyLimit | OrderType::BuyStop
        )
    }

    pub fn is_sell(self) -> bool {
        matches!(
            self,
            OrderType::Sell | OrderType::SellLimit | OrderType::SellStop
        )
    }
}

/// Complete lifecycle state of an order in the execution state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderState {
    /// Initial state before network transmission.
    Created,
    /// Actively transmitting or awaiting confirmation.
    Submitting,
    /// Pending order accepted and working in the MT5 order book.
    Accepted,
    /// Order partially executed; working remainder remains.
    PartiallyFilled,
    /// Order completely executed.
    Filled,
    /// Order was cancelled or expired.
    Cancelled,
    /// Order was rejected by the bridge or broker.
    Rejected,
    /// Ambiguous outcome (e.g. timeout / disconnect after submission).
    /// Requires reconciliation against broker state before any retry!
    Unknown,
    /// Order confirmed and reconstructed via reconciliation.
    Reconciled,
    /// Reconciliation found the order on the broker, but one or more attributes
    /// (volume, direction, SL/TP, symbol, magic…) differ from the original request.
    Mismatched,
}

/// Trade execution status classification (high-level convenience classification).
///
/// **Authoritative Semantics:**
/// The authoritative field for MT5 execution outcome is always [`TradeResult::retcode`](crate::types::trade_result::TradeResult::retcode).
/// This enum provides an ergonomic classification of common MT5 outcomes:
/// - `Filled`: Market order executed immediately with a deal ticket (`TRADE_RETCODE_DONE` = 10009, `deal > 0`).
/// - `Placed`: Pending order accepted and currently working in the terminal (`TRADE_RETCODE_PLACED` = 10008,
///   or 10009 with `deal == 0` and `order > 0`).
/// - `PartiallyFilled`: Order partially executed (`TRADE_RETCODE_DONE_PARTIAL` = 10010).
/// - `Rejected`: Order rejected or failed (any error retcode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeStatus {
    /// Request completed in full (`TRADE_RETCODE_DONE` = 10009 with `deal > 0`).
    Filled,
    /// Pending order placed and working (`TRADE_RETCODE_PLACED` = 10008 or 10009 with `deal == 0`).
    Placed,
    /// Only part of the requested volume was filled (`TRADE_RETCODE_DONE_PARTIAL` = 10010).
    PartiallyFilled,
    /// Order was rejected or failed.
    Rejected,
}

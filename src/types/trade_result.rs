use crate::error::mt5_retcode_description;
use crate::ffi::Mt5TradeResult;
use crate::types::order_enums::TradeStatus;
use serde::{Deserialize, Serialize};

/// Result returned from an order placement or closure.
///
/// **Authoritative Outcome:**
/// The authoritative status of the order is [`retcode`](Self::retcode).
/// Use [`is_deal()`](Self::is_deal) to check if a deal was executed immediately,
/// [`is_working_order()`](Self::is_working_order) to check if a pending order is active,
/// and [`has_position()`](Self::has_position) to check if a position ticket was assigned.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TradeResult {
    /// MT5 Trade execution return code (e.g. 10009 for `TRADE_RETCODE_DONE`).
    /// This is the authoritative raw status code returned by MetaTrader 5.
    pub retcode: u32,
    /// Deal ticket number if a deal was executed (0 for pending orders).
    pub deal: u64,
    /// Order ticket number.
    pub order: u64,
    /// Position ticket number associated with the trade (0 if pending or unknown).
    pub position: u64,
    /// Executed trade volume.
    pub volume: f64,
    /// Execution price.
    pub price: f64,
}

impl TradeResult {
    pub fn from_raw(raw: Mt5TradeResult) -> Self {
        Self {
            retcode: raw.retcode,
            deal: raw.deal,
            order: raw.order,
            position: raw.position,
            volume: raw.volume,
            price: raw.price,
        }
    }

    /// Classified status of the trade execution.
    pub fn status(&self) -> TradeStatus {
        match self.retcode {
            10009 => {
                if self.deal > 0 {
                    TradeStatus::Filled
                } else if self.order > 0 {
                    TradeStatus::Placed
                } else {
                    TradeStatus::Filled
                }
            }
            10008 => TradeStatus::Placed,
            10010 => TradeStatus::PartiallyFilled,
            _ => TradeStatus::Rejected,
        }
    }

    /// Returns `true` if the order was successfully completed, placed, or partially filled.
    pub fn is_success(&self) -> bool {
        matches!(
            self.status(),
            TradeStatus::Filled | TradeStatus::Placed | TradeStatus::PartiallyFilled
        )
    }

    /// Returns `true` if the order was executed in full as an immediate deal.
    pub fn is_filled(&self) -> bool {
        self.status() == TradeStatus::Filled
    }

    /// Returns `true` if a pending order was placed and is working.
    pub fn is_placed(&self) -> bool {
        self.status() == TradeStatus::Placed
    }

    /// Returns `true` if only part of the requested volume was filled.
    pub fn is_partially_filled(&self) -> bool {
        self.status() == TradeStatus::PartiallyFilled
    }

    /// Returns `true` if a deal was executed immediately (`deal > 0`).
    pub fn is_deal(&self) -> bool {
        self.deal > 0
    }

    /// Returns `true` if this result represents a working pending order (`deal == 0 && order > 0`).
    pub fn is_working_order(&self) -> bool {
        self.deal == 0 && self.order > 0
    }

    /// Returns `true` if a non-zero position ticket is present.
    pub fn has_position(&self) -> bool {
        self.position > 0
    }

    /// Human-readable explanation of the return code.
    pub fn description(&self) -> &'static str {
        mt5_retcode_description(self.retcode)
    }
}

use crate::types::order_enums::OrderType;
use serde::{Deserialize, Serialize};

/// Persisted trade-management rule attached to a filled order.
///
/// Created when an order reaches [`OrderState::Filled`]; updated on every
/// break-even or trailing-stop modification; removed when the position is closed.
///
/// All fields are owned and serializable so the rule can be journaled to the WAL
/// and reconstructed after a process restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeRule {
    /// The client order ID this rule belongs to.
    pub client_order_id: String,
    /// Symbol the rule trades.
    pub symbol: String,
    /// Position ticket (filled market orders) or order ticket (pending orders)
    /// used for `order_modify_with_magic`.
    pub ticket: u64,
    /// Order direction.
    pub order_type: OrderType,
    /// Entry price (average fill price).
    pub entry_price: f64,
    /// Order volume.
    pub volume: f64,
    /// Current stop-loss price (`0.0` = none).
    pub stop_loss: f64,
    /// Current take-profit price (`0.0` = none).
    pub take_profit: f64,
    /// Break-even trigger: activate when profit >= `break_even_at_r * risk`.
    /// `None` disables break-even.
    pub break_even_at_r: Option<f64>,
    /// Trailing distance from current price (price units). `None` disables trailing.
    pub trailing_distance: Option<f64>,
    /// Minimum improvement (price units) required before modifying.
    pub trailing_step: Option<f64>,
    /// Trailing activation: activate when profit >= `trailing_activation * risk`.
    /// `None` disables the activation check.
    pub trailing_activation: Option<f64>,
    /// Risk distance (entry − SL for buy, SL − entry for sell) in price units.
    /// Used to compute R-multiples for break-even and trailing activation.
    pub risk: f64,
    /// Symbol `SYMBOL_TRADE_STOPS_LEVEL`: the new SL must be at least this far
    /// from the current market price. `0.0` disables the check.
    pub stops_level: f64,
    /// Epoch seconds of the last successful modification (`0` = never).
    /// Used for the one-second rate limit; reset on reload.
    pub last_modify_secs: i64,
    /// Whether break-even has already been activated.
    pub break_even_activated: bool,
    /// Whether trailing has already been activated.
    pub trailing_activated: bool,
}

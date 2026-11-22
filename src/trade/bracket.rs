use crate::types::{Level, OrderRequest, TradeRule};

/// A complete bracket order: entry plus protective stop-loss and take-profit,
/// with optional break-even and trailing-stop management.
///
/// Converted into a [`TradeRule`] once the entry order fills, at which point
/// [`TradeManager`](crate::trade::TradeManager) takes over SL/TP adjustments.
#[derive(Debug, Clone)]
pub struct Bracket {
    /// The entry order request.
    pub entry: OrderRequest,
    /// Initial stop-loss level.
    pub stop_loss: Level,
    /// Initial take-profit level.
    pub take_profit: Level,
    /// Break-even trigger in R multiples; `None` disables break-even.
    pub break_even_at_r: Option<f64>,
    /// Optional trailing-stop configuration.
    pub trailing: Option<super::TrailingStop>,
}

impl Bracket {
    /// Convert this bracket into a [`TradeRule`] once the entry order has filled.
    ///
    /// `entry_price` is the average fill price. `ticket` is the position or order
    /// ticket used for modifications. `risk` is the entry-to-SL distance in price units.
    pub fn into_rule(
        self,
        client_order_id: String,
        symbol: String,
        ticket: u64,
        entry_price: f64,
        risk: f64,
    ) -> TradeRule {
        let (trailing_distance, trailing_step, trailing_activation) =
            self.trailing.map_or((None, None, None), |t| {
                (Some(t.distance), Some(t.step), Some(t.activation))
            });

        TradeRule {
            client_order_id,
            symbol,
            ticket,
            order_type: self.entry.order_type,
            entry_price,
            volume: self.entry.volume,
            stop_loss: match self.stop_loss {
                Level::Set(v) => v,
                _ => 0.0,
            },
            take_profit: match self.take_profit {
                Level::Set(v) => v,
                _ => 0.0,
            },
            break_even_at_r: self.break_even_at_r,
            trailing_distance,
            trailing_step,
            trailing_activation,
            risk,
            stops_level: 0.0,
            last_modify_secs: 0,
            break_even_activated: false,
            trailing_activated: false,
        }
    }
}

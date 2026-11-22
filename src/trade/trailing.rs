use crate::types::TradeRule;

/// Trailing-stop configuration for a [`TradeRule`].
///
/// Trailing activates when profit reaches `activation * risk`; from then on the
/// stop is trailed at `distance` from the current price, moving only when the
/// improvement exceeds `step`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailingStop {
    /// Distance from current price for the new stop level (price units).
    pub distance: f64,
    /// Minimum improvement (price units) required before modifying SL.
    pub step: f64,
    /// Profit threshold in R multiples at which trailing activates.
    pub activation: f64,
}

impl TrailingStop {
    /// Compute the candidate trailing stop-loss price for `rule` at `price`.
    ///
    /// Returns `None` when trailing is not active for this rule or the candidate
    /// does not improve the existing stop (SL only moves up for buys, down for sells).
    pub fn candidate_sl(&self, rule: &TradeRule, price: f64) -> Option<f64> {
        if !rule.trailing_activated {
            return None;
        }
        let is_buy = rule.order_type.is_buy();
        let candidate = if is_buy {
            price - self.distance
        } else {
            price + self.distance
        };
        let better = if is_buy {
            candidate > rule.stop_loss
        } else {
            candidate < rule.stop_loss || rule.stop_loss == 0.0
        };
        better.then_some(candidate)
    }
}

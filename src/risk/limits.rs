use std::collections::HashSet;

/// Portfolio-level pre-trade risk limits.
///
/// Limits set to `0` or `None` are disabled. Limits are not persisted; they are
/// configuration supplied at runtime by the strategy.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct RiskLimits {
    /// Maximum number of open positions across all symbols. `0` disables.
    pub max_open_positions: usize,
    /// Maximum total volume per symbol (sum of open positions + working orders).
    /// `0.0` disables.
    pub max_volume_per_symbol: f64,
    /// Maximum total volume across all symbols. `0.0` disables.
    pub max_total_volume: f64,
    /// Maximum realized loss for the trading day (deposit currency). `0.0` disables.
    /// Resets at `daily_reset_hour` UTC.
    pub max_daily_loss: f64,
    /// Maximum drawdown from day-start equity, as a fraction (e.g. `0.10` = 10%).
    /// `0.0` disables.
    pub max_drawdown_pct: f64,
    /// Whitelist of allowed symbols. `None` allows all symbols not in `blocked_symbols`.
    pub allowed_symbols: Option<HashSet<String>>,
    /// Symbols that are never tradable.
    pub blocked_symbols: HashSet<String>,
    /// Maximum number of order submissions per minute. `0` disables.
    pub max_orders_per_minute: u32,
}

impl RiskLimits {
    /// Returns `true` if every limit is disabled (default configuration).
    pub fn is_disabled(&self) -> bool {
        self.max_open_positions == 0
            && self.max_volume_per_symbol == 0.0
            && self.max_total_volume == 0.0
            && self.max_daily_loss == 0.0
            && self.max_drawdown_pct == 0.0
            && self.allowed_symbols.is_none()
            && self.blocked_symbols.is_empty()
            && self.max_orders_per_minute == 0
    }
}

use serde::{Deserialize, Serialize};

/// MetaTrader 5 account balance, equity, and margin details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AccountInfo {
    /// Account balance in deposit currency.
    pub balance: f64,
    /// Account equity in deposit currency.
    pub equity: f64,
    /// Currently reserved margin.
    pub margin: f64,
    /// Free margin available for opening new positions.
    pub free_margin: f64,
    /// The account leverage, when the terminal reports it. `0` when unknown.
    pub leverage: f64,
    /// The deposit currency of the account, when the terminal reports it.
    pub currency: String,
}

impl AccountInfo {
    /// Floating profit or loss (`equity - balance`).
    pub fn profit(&self) -> f64 {
        self.equity - self.balance
    }

    /// Current margin level percentage (`equity / margin * 100.0`), or `None` if margin is zero.
    pub fn margin_level(&self) -> Option<f64> {
        if self.margin > 0.0 {
            Some((self.equity / self.margin) * 100.0)
        } else {
            None
        }
    }
}

/// Result of a heartbeat ping to MT5 EA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingResult {
    /// Current server time reported by MT5 terminal in UTC seconds.
    pub server_time_secs: i64,
    /// Wire protocol version negotiated with the EA.
    pub protocol: u32,
    /// Monotonic microsecond counter reported by the EA (`CMD_PING`).
    pub monotonic_microseconds: u64,
    /// Round-trip time measured on the Rust side, in microseconds.
    pub rtt_us: u64,
}

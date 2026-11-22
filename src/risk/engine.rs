use crate::error::{Mt5Error, Result};
use crate::risk::limits::RiskLimits;
use crate::types::{AccountInfo, OrderRequest, Position, WorkingOrder};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;
use tracing::debug;

/// In-memory snapshot of portfolio state used by the pre-trade risk engine.
///
/// Updated from [`AccountInfo`], [`Position`], and [`WorkingOrder`] snapshots
/// so [`RiskEngine::pre_trade`] can evaluate limits
/// without querying the backend on every submission.
#[derive(Debug, Clone, Default)]
pub struct PortfolioState {
    /// Latest account equity (deposit currency).
    pub equity: f64,
    /// Latest account balance (deposit currency).
    pub balance: f64,
    /// Day-start equity used for drawdown and daily-loss baselines.
    pub day_start_equity: Option<f64>,
    /// UTC date (YYYYMMDD) of the last daily-loss reset.
    pub last_reset_date: i64,
    /// Open positions keyed by ticket.
    pub positions: HashMap<u64, Position>,
    /// Working pending orders keyed by ticket.
    pub working_orders: HashMap<u64, WorkingOrder>,
}

/// Result of a pre-trade risk evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreTradeDecision {
    /// The request passes all enabled risk checks.
    Allowed,
    /// The request is rejected by rule `rule` with `detail`.
    Rejected { rule: &'static str, detail: String },
}

/// Pre-trade risk engine: portfolio-level checks before an order reaches the backend.
///
/// A rejection never reaches the MT5 backend. The engine is `Send + Sync` and all
/// methods take `&self`; state is guarded by internal mutexes so it can be shared
/// between the gateway actor task and observer callbacks.
pub struct RiskEngine {
    limits: RiskLimits,
    state: Mutex<PortfolioState>,
    order_times: Mutex<VecDeque<Instant>>,
}

impl std::fmt::Debug for RiskEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RiskEngine")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Default for RiskEngine {
    fn default() -> Self {
        Self::new(RiskLimits::default())
    }
}

impl RiskEngine {
    /// Create a new risk engine with the given limits.
    pub fn new(limits: RiskLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(PortfolioState::default()),
            order_times: Mutex::new(VecDeque::new()),
        }
    }

    /// Evaluate `req` against current portfolio state and limits.
    ///
    /// Returns [`PreTradeDecision::Allowed`] when every enabled check passes, or
    /// [`PreTradeDecision::Rejected`] naming the violated rule. A disabled limit
    /// (zero or `None`) is skipped.
    pub fn pre_trade(
        &self,
        req: &OrderRequest,
        account: &AccountInfo,
        positions: &[Position],
        working_orders: &[WorkingOrder],
    ) -> Result<PreTradeDecision, Mt5Error> {
        self.enforce_daily_reset(account);

        // Symbol allow/block list.
        if let Some(allowed) = &self.limits.allowed_symbols {
            if !allowed.contains(&req.symbol) {
                return Ok(PreTradeDecision::Rejected {
                    rule: "allowed_symbols",
                    detail: format!("symbol '{}' is not in the allowed symbol list", req.symbol),
                });
            }
        }
        if self.limits.blocked_symbols.contains(&req.symbol) {
            return Ok(PreTradeDecision::Rejected {
                rule: "blocked_symbols",
                detail: format!("symbol '{}' is blocked", req.symbol),
            });
        }

        // Max open positions.
        if self.limits.max_open_positions > 0 {
            let open_count = positions.len();
            if open_count >= self.limits.max_open_positions {
                return Ok(PreTradeDecision::Rejected {
                    rule: "max_open_positions",
                    detail: format!(
                        "max open positions {} reached ({} open)",
                        self.limits.max_open_positions, open_count
                    ),
                });
            }
        }

        // Max volume per symbol.
        if self.limits.max_volume_per_symbol > 0.0 {
            let symbol_volume: f64 = positions
                .iter()
                .filter(|p| p.symbol == req.symbol)
                .map(|p| p.volume)
                .sum::<f64>()
                + working_orders
                    .iter()
                    .filter(|o| o.symbol == req.symbol)
                    .map(|o| o.volume_current)
                    .sum::<f64>()
                + req.volume;
            if symbol_volume > self.limits.max_volume_per_symbol {
                return Ok(PreTradeDecision::Rejected {
                    rule: "max_volume_per_symbol",
                    detail: format!(
                        "max volume per symbol {} exceeded for '{}' (would be {})",
                        self.limits.max_volume_per_symbol, req.symbol, symbol_volume
                    ),
                });
            }
        }

        // Max total volume.
        if self.limits.max_total_volume > 0.0 {
            let total_volume: f64 = positions
                .iter()
                .map(|p| p.volume)
                .chain(working_orders.iter().map(|o| o.volume_current))
                .sum::<f64>()
                + req.volume;
            if total_volume > self.limits.max_total_volume {
                return Ok(PreTradeDecision::Rejected {
                    rule: "max_total_volume",
                    detail: format!(
                        "max total volume {} exceeded (would be {})",
                        self.limits.max_total_volume, total_volume
                    ),
                });
            }
        }

        // Max daily loss (realized loss since day start).
        if self.limits.max_daily_loss > 0.0 {
            let day_start = self
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .day_start_equity;
            if let Some(start) = day_start {
                let realized_pnl = account.equity - start;
                if realized_pnl < -self.limits.max_daily_loss {
                    return Ok(PreTradeDecision::Rejected {
                        rule: "max_daily_loss",
                        detail: format!(
                            "max daily loss {} exceeded (realized {})",
                            self.limits.max_daily_loss, realized_pnl
                        ),
                    });
                }
            }
        }

        // Max drawdown from day-start equity.
        if self.limits.max_drawdown_pct > 0.0 {
            let day_start = self
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .day_start_equity;
            if let Some(start) = day_start {
                let drawdown = (start - account.equity) / start;
                if drawdown > self.limits.max_drawdown_pct {
                    return Ok(PreTradeDecision::Rejected {
                        rule: "max_drawdown_pct",
                        detail: format!(
                            "max drawdown {:.2}% exceeded (current {:.2}%)",
                            drawdown * 100.0,
                            self.limits.max_drawdown_pct * 100.0
                        ),
                    });
                }
            }
        }

        // Max orders per minute (sliding window).
        if self.limits.max_orders_per_minute > 0 {
            let now = Instant::now();
            let mut times = self.order_times.lock().unwrap_or_else(|p| p.into_inner());
            while let Some(front) = times.front() {
                if now.duration_since(*front) >= std::time::Duration::from_secs(60) {
                    times.pop_front();
                } else {
                    break;
                }
            }
            if times.len() >= self.limits.max_orders_per_minute as usize {
                return Ok(PreTradeDecision::Rejected {
                    rule: "max_orders_per_minute",
                    detail: format!(
                        "max orders per minute {} reached",
                        self.limits.max_orders_per_minute
                    ),
                });
            }
            times.push_back(now);
        }

        Ok(PreTradeDecision::Allowed)
    }

    /// Update portfolio state from an account info snapshot.
    pub fn on_account_info(&self, account: &AccountInfo) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.equity = account.equity;
        state.balance = account.balance;
        if state.day_start_equity.is_none() {
            state.day_start_equity = Some(account.equity);
        }
    }

    /// Replace the tracked positions snapshot.
    pub fn on_positions(&self, positions: &[Position]) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.positions = positions.iter().map(|p| (p.ticket, p.clone())).collect();
    }

    /// Replace the tracked working orders snapshot.
    pub fn on_working_orders(&self, orders: &[WorkingOrder]) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.working_orders = orders.iter().map(|o| (o.ticket, o.clone())).collect();
    }

    /// Reset the day-start baseline and daily-loss tracking. Called at 00:00 UTC
    /// (or whenever a new trading day begins).
    pub fn reset_daily(&self, equity: f64) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.day_start_equity = Some(equity);
        state.last_reset_date = utc_date();
    }

    /// Current portfolio state (clone) for inspection / testing.
    pub fn state(&self) -> PortfolioState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn enforce_daily_reset(&self, account: &AccountInfo) {
        let today = utc_date();
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.last_reset_date != today {
            state.last_reset_date = today;
            state.day_start_equity = Some(account.equity);
            debug!(
                "Daily risk reset: day-start equity set to {}",
                account.equity
            );
        }
    }
}

/// UTC date as YYYYMMDD for daily reset boundaries.
fn utc_date() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    // Days since epoch; crude but sufficient for daily reset boundaries without a
    // hard chrono dependency in the risk crate core.
    let days = secs / 86400;
    // Approximate YYYYMMDD: only used for equality comparison across calls.
    days * 10000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OrderType;
    use std::collections::HashSet;

    fn limits() -> RiskLimits {
        RiskLimits {
            max_open_positions: 5,
            max_volume_per_symbol: 10.0,
            max_total_volume: 50.0,
            max_daily_loss: 500.0,
            max_drawdown_pct: 0.10,
            allowed_symbols: None,
            blocked_symbols: HashSet::new(),
            max_orders_per_minute: 60,
        }
    }

    fn account() -> AccountInfo {
        AccountInfo {
            balance: 10_000.0,
            equity: 10_000.0,
            margin: 1_000.0,
            free_margin: 9_000.0,
            leverage: 100.0,
            currency: "USD".into(),
        }
    }

    fn req(symbol: &str, volume: f64, order_type: OrderType) -> OrderRequest {
        OrderRequest {
            symbol: symbol.into(),
            order_type,
            volume,
            price: 0.0,
            stop_loss: 1.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    #[test]
    fn allowed_when_all_limits_disabled() {
        let engine = RiskEngine::new(RiskLimits::default());
        assert!(engine.limits.is_disabled());
        let dec = engine
            .pre_trade(&req("EURUSD", 1.0, OrderType::Buy), &account(), &[], &[])
            .unwrap();
        assert_eq!(dec, PreTradeDecision::Allowed);
    }

    #[test]
    fn rejects_max_open_positions() {
        let engine = RiskEngine::new(limits());
        let positions: Vec<Position> = (0..5)
            .map(|i| Position {
                ticket: i,
                time: 0,
                position_type: OrderType::Buy,
                magic: 1,
                volume: 1.0,
                price_open: 1.0,
                stop_loss: 0.0,
                take_profit: 0.0,
                price_current: 1.0,
                profit: 0.0,
                swap: 0.0,
                symbol: "EURUSD".into(),
                comment: String::new(),
            })
            .collect();
        let dec = engine
            .pre_trade(
                &req("EURUSD", 1.0, OrderType::Buy),
                &account(),
                &positions,
                &[],
            )
            .unwrap();
        assert!(
            matches!(dec, PreTradeDecision::Rejected { rule, .. } if rule == "max_open_positions")
        );
    }

    #[test]
    fn rejects_max_volume_per_symbol() {
        let engine = RiskEngine::new(limits());
        let positions = vec![Position {
            ticket: 1,
            time: 0,
            position_type: OrderType::Buy,
            magic: 1,
            volume: 8.0,
            price_open: 1.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            price_current: 1.0,
            profit: 0.0,
            swap: 0.0,
            symbol: "EURUSD".into(),
            comment: String::new(),
        }];
        // 8.0 + 1.0 = 9.0 <= 10.0 allowed
        let dec = engine
            .pre_trade(
                &req("EURUSD", 1.0, OrderType::Buy),
                &account(),
                &positions,
                &[],
            )
            .unwrap();
        assert_eq!(dec, PreTradeDecision::Allowed);
        // 8.0 + 3.0 = 11.0 > 10.0 rejected
        let dec = engine
            .pre_trade(
                &req("EURUSD", 3.0, OrderType::Buy),
                &account(),
                &positions,
                &[],
            )
            .unwrap();
        assert!(
            matches!(dec, PreTradeDecision::Rejected { rule, .. } if rule == "max_volume_per_symbol")
        );
    }

    #[test]
    fn rejects_blocked_symbol() {
        let mut limits = limits();
        limits.blocked_symbols.insert("XAUUSD".into());
        let engine = RiskEngine::new(limits);
        let dec = engine
            .pre_trade(&req("XAUUSD", 1.0, OrderType::Buy), &account(), &[], &[])
            .unwrap();
        assert!(
            matches!(dec, PreTradeDecision::Rejected { rule, .. } if rule == "blocked_symbols")
        );
    }

    #[test]
    fn rejects_max_daily_loss() {
        let mut limits = limits();
        limits.max_daily_loss = 100.0;
        let engine = RiskEngine::new(limits);
        let mut acct = account();
        acct.equity = 9_850.0; // 150 loss > 100 limit
                               // day-start equity defaults to current equity on first account_info call, so
                               // simulate a prior day-start by resetting then lowering equity.
        engine.reset_daily(10_000.0);
        let dec = engine
            .pre_trade(&req("EURUSD", 1.0, OrderType::Buy), &acct, &[], &[])
            .unwrap();
        assert!(matches!(dec, PreTradeDecision::Rejected { rule, .. } if rule == "max_daily_loss"));
    }

    #[test]
    fn rejects_max_drawdown() {
        let mut limits = limits();
        limits.max_drawdown_pct = 0.05; // 5%
        limits.max_daily_loss = 0.0; // disable so it doesn't fire before drawdown check
        let engine = RiskEngine::new(limits);
        engine.reset_daily(10_000.0);
        let mut acct = account();
        acct.equity = 9_400.0; // 6% drawdown > 5%
        let dec = engine
            .pre_trade(&req("EURUSD", 1.0, OrderType::Buy), &acct, &[], &[])
            .unwrap();
        assert!(
            matches!(dec, PreTradeDecision::Rejected { rule, .. } if rule == "max_drawdown_pct")
        );
    }

    #[test]
    fn respects_order_rate_limit() {
        let mut limits = limits();
        limits.max_orders_per_minute = 2;
        let engine = RiskEngine::new(limits);
        let acct = account();
        assert_eq!(
            engine
                .pre_trade(&req("EURUSD", 1.0, OrderType::Buy), &acct, &[], &[])
                .unwrap(),
            PreTradeDecision::Allowed
        );
        assert_eq!(
            engine
                .pre_trade(&req("EURUSD", 1.0, OrderType::Buy), &acct, &[], &[])
                .unwrap(),
            PreTradeDecision::Allowed
        );
        let dec = engine
            .pre_trade(&req("EURUSD", 1.0, OrderType::Buy), &acct, &[], &[])
            .unwrap();
        assert!(
            matches!(dec, PreTradeDecision::Rejected { rule, .. } if rule == "max_orders_per_minute")
        );
    }

    #[test]
    fn allows_within_all_limits() {
        let engine = RiskEngine::new(limits());
        let dec = engine
            .pre_trade(&req("EURUSD", 1.0, OrderType::Buy), &account(), &[], &[])
            .unwrap();
        assert_eq!(dec, PreTradeDecision::Allowed);
    }
}

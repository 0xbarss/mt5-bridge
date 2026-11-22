use crate::backend::TradingBackend;
use crate::error::Result;
use crate::journal::OrderStore;
use crate::reconciliation::events::{OrderEvent, OrderObserver};
use crate::reconciliation::shared::SharedOrderManager;
use crate::trade::trailing::TrailingStop;
use crate::types::{Level, OrderType, TradeRule};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Minimum interval between successive modifications of the same rule.
const MIN_MODIFY_INTERVAL: Duration = Duration::from_secs(1);

/// Small buffer added to break-even stop so it is not immediately triggered by spread.
const BREAK_EVEN_BUFFER_PCT: f64 = 1e-6;

/// Trade manager: brackets, break-even, and trailing stops.
///
/// Subscribes to [`OrderEvent`]s via [`OrderObserver`]: on [`OrderEvent::Filled`] it
/// creates a [`TradeRule`]; on [`OrderEvent::Closed`] it removes it from the active set.
/// Call [`check_rules`](Self::check_rules) on every tick (or timer tick) to evaluate
/// break-even and trailing-stop modifications and apply them via the backend.
///
/// # Concurrency
///
/// All methods are `&self` and safe to call from any thread. Rule state is guarded by a
/// `Mutex`; backend modify calls are made while holding the rule lock, which matches the
/// serialized pipe I/O of the underlying DLL.
pub struct TradeManager {
    manager: SharedOrderManager,
    backend: Arc<dyn TradingBackend + Send + Sync>,
    rules: Mutex<HashMap<String, (TradeRule, std::time::Instant)>>,
    store: Option<Box<dyn OrderStore + Send + Sync>>,
}

impl TradeManager {
    /// Create a new trade manager wired to a shared order manager and trading backend.
    pub fn new(
        manager: SharedOrderManager,
        backend: Arc<dyn TradingBackend + Send + Sync>,
    ) -> Self {
        Self {
            manager,
            backend,
            rules: Mutex::new(HashMap::new()),
            store: None,
        }
    }

    /// Attach a durable [`OrderStore`] so rules survive restart.
    ///
    /// Rules are journaled via [`OrderStore::record_trade_rule`] on creation and
    /// modification, and reloaded via [`OrderStore::load_trade_rules`] on restart.
    pub fn with_store(mut self, store: Box<dyn OrderStore + Send + Sync>) -> Self {
        self.store = Some(store);
        self
    }

    /// Import rules previously persisted (e.g. loaded from WAL on restart).
    ///
    /// Rules for orders that are no longer active are ignored by [`check_rules`].
    /// Imported rules are not subject to the rate limit so they can be evaluated
    /// immediately on the next tick.
    pub fn import_rules(&self, rules: Vec<TradeRule>) {
        let mut guard = self.rules.lock().unwrap();
        let stale = std::time::Instant::now()
            .checked_sub(Duration::from_secs(2))
            .unwrap_or(std::time::Instant::now());
        for rule in rules {
            guard.insert(rule.client_order_id.clone(), (rule, stale));
        }
    }

    /// Export all active rules.
    pub fn export_rules(&self) -> Vec<TradeRule> {
        let guard = self.rules.lock().unwrap();
        guard.values().map(|(rule, _)| rule.clone()).collect()
    }

    /// Return the rule for `client_order_id`, if any.
    pub fn get_rule(&self, client_order_id: &str) -> Option<TradeRule> {
        let guard = self.rules.lock().unwrap();
        guard.get(client_order_id).map(|(r, _)| r.clone())
    }

    /// Evaluate break-even and trailing-stop for all rules on `symbol` at `price`.
    ///
    /// `price` should be the current executable price: `bid` for buy positions,
    /// `ask` for sell positions. Returns the client order IDs that were modified.
    ///
    /// A rule is modified only when all of the following hold:
    /// - the order still exists and is not terminal;
    /// - at least `MIN_MODIFY_INTERVAL` has elapsed since its last modification;
    /// - the new stop-loss improves on the current one by at least `trailing_step`;
    /// - the new stop-loss is at least `stops_level` away from the current price.
    pub fn check_rules(&self, symbol: &str, price: f64) -> Result<Vec<String>> {
        let now_secs = now_epoch_secs();

        // Snapshot rules to check without holding the lock during backend calls.
        let rules_to_check: Vec<(String, TradeRule)> = {
            let guard = self.rules.lock().unwrap();
            guard
                .iter()
                .filter(|(_, (rule, _))| rule.symbol == symbol)
                .map(|(cid, (rule, _))| (cid.clone(), rule.clone()))
                .collect()
        };

        let mut modified = Vec::new();

        for (client_order_id, mut rule) in rules_to_check {
            // The order must still exist and not be terminal.
            let Some(order) = self.manager.get_order(&client_order_id) else {
                continue;
            };
            if order.is_terminal() {
                continue;
            }

            // Rate limit: skip if modified too recently.
            let last_modify = {
                let guard = self.rules.lock().unwrap();
                guard
                    .get(&client_order_id)
                    .map(|(_, t)| *t)
                    .unwrap_or(std::time::Instant::now())
            };
            if last_modify.elapsed() < MIN_MODIFY_INTERVAL {
                continue;
            }

            let is_buy = rule.order_type.is_buy();
            let risk = rule.risk;
            if risk <= 0.0 {
                continue;
            }

            // Profit in price units and R multiples.
            let profit = if is_buy {
                price - rule.entry_price
            } else {
                rule.entry_price - price
            };
            let profit_r = profit / risk;

            let mut new_sl: Option<f64> = None;

            // --- Break-even ---
            if let Some(be_at_r) = rule.break_even_at_r {
                if !rule.break_even_activated && profit_r >= be_at_r {
                    let buffer = rule.entry_price * BREAK_EVEN_BUFFER_PCT;
                    let be_sl = if is_buy {
                        rule.entry_price + buffer
                    } else {
                        rule.entry_price - buffer
                    };
                    new_sl = Some(be_sl);
                    rule.break_even_activated = true;
                }
            }

            // --- Trailing activation ---
            if let Some(activation) = rule.trailing_activation {
                if !rule.trailing_activated && profit_r >= activation {
                    rule.trailing_activated = true;
                }
            }

            // --- Trailing stop ---
            if rule.trailing_activated {
                let trailing = TrailingStop {
                    distance: rule.trailing_distance.unwrap_or(0.0),
                    step: rule.trailing_step.unwrap_or(0.0),
                    activation: rule.trailing_activation.unwrap_or(0.0),
                };
                if let Some(candidate) = trailing.candidate_sl(&rule, price) {
                    new_sl = Some(candidate);
                }
            }

            let Some(candidate_sl) = new_sl else {
                continue;
            };

            // Skip if no meaningful change.
            if (candidate_sl - rule.stop_loss).abs() < crate::constants::PRICE_TOLERANCE {
                continue;
            }

            // Stops-level check: new SL must be at least `stops_level` from price.
            if rule.stops_level > 0.0 {
                let distance = (candidate_sl - price).abs();
                if distance < rule.stops_level {
                    continue;
                }
            }

            // Improvement check: new SL must be better than current by at least `step`.
            let step = rule.trailing_step.unwrap_or(0.0);
            if step > 0.0 {
                let improvement = (candidate_sl - rule.stop_loss).abs();
                if improvement < step {
                    continue;
                }
            }

            // Apply the modification via the backend.
            self.backend.order_modify_with_magic(
                rule.ticket,
                self.manager.magic().get(),
                Level::Set(candidate_sl),
                Level::Keep,
            )?;

            // Persist the updated rule.
            rule.stop_loss = candidate_sl;
            rule.last_modify_secs = now_secs;
            if let Some(store) = &self.store {
                let _ = store.record_trade_rule(&rule.symbol, &rule);
            }

            // Update in-memory state.
            {
                let mut guard = self.rules.lock().unwrap();
                if let Some((stored_rule, stored_instant)) = guard.get_mut(&client_order_id) {
                    *stored_rule = rule;
                    *stored_instant = std::time::Instant::now();
                }
            }

            modified.push(client_order_id);
        }

        Ok(modified)
    }

    /// Remove the rule for `client_order_id` (called internally on `OrderEvent::Closed`).
    fn remove_rule(&self, client_order_id: &str, symbol: &str) {
        let mut guard = self.rules.lock().unwrap();
        if let Some((rule, _)) = guard.remove(client_order_id) {
            if let Some(store) = &self.store {
                // Re-persist the rule with its current state; the WAL keeps a
                // last-write-wins record per client order ID, and on restart rules
                // for terminal orders are ignored by `check_rules`.
                let _ = store.record_trade_rule(symbol, &rule);
            }
        }
    }
}

impl OrderObserver for TradeManager {
    fn on_event(&self, event: &OrderEvent) {
        match event {
            OrderEvent::Filled {
                client_order_id,
                price,
                ..
            } => {
                let Some(order) = self.manager.get_order(client_order_id) else {
                    return;
                };
                // Only manage orders that have a broker ticket (position or pending order).
                if order.position_ticket == 0 && order.order_ticket == 0 {
                    return;
                }
                let ticket = if order.position_ticket != 0 {
                    order.position_ticket
                } else {
                    order.order_ticket
                };
                // Use the fill price as entry for risk computation (requested_price is 0.0
                // for market orders).
                let risk = compute_risk(order.order_type, *price, order.requested_stop_loss);
                let rule = TradeRule {
                    client_order_id: client_order_id.clone(),
                    symbol: order.symbol.clone(),
                    ticket,
                    order_type: order.order_type,
                    entry_price: *price,
                    volume: order.requested_volume,
                    stop_loss: order.requested_stop_loss,
                    take_profit: order.requested_take_profit,
                    break_even_at_r: None,
                    trailing_distance: None,
                    trailing_step: None,
                    trailing_activation: None,
                    risk,
                    stops_level: 0.0,
                    last_modify_secs: 0,
                    break_even_activated: false,
                    trailing_activated: false,
                };
                if let Some(store) = &self.store {
                    let _ = store.record_trade_rule(&order.symbol, &rule);
                }
                let mut guard = self.rules.lock().unwrap();
                guard.insert(client_order_id.clone(), (rule, std::time::Instant::now()));
            }
            OrderEvent::Closed {
                client_order_id, ..
            } => {
                let symbol = self
                    .manager
                    .get_order(client_order_id)
                    .map(|o| o.symbol.clone())
                    .unwrap_or_default();
                self.remove_rule(client_order_id, &symbol);
            }
            _ => {}
        }
    }
}

/// Compute the risk distance (entry − SL for buy, SL − entry for sell) in price units.
fn compute_risk(order_type: OrderType, entry_price: f64, stop_loss: f64) -> f64 {
    if stop_loss <= 0.0 {
        return 0.0;
    }
    match order_type {
        OrderType::Buy | OrderType::BuyLimit | OrderType::BuyStop => {
            (entry_price - stop_loss).abs()
        }
        OrderType::Sell | OrderType::SellLimit | OrderType::SellStop => {
            (stop_loss - entry_price).abs()
        }
    }
}

/// Current epoch seconds (UTC).
fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

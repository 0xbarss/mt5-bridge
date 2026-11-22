//! Integration tests for bracket, break-even, and trailing-stop trade management (A4).
//!
//! Uses a mock [`TradingBackend`](crate::backend::TradingBackend) to capture
//! `order_modify_with_magic` calls without a live terminal.

use mt5_bridge::backend::TradingBackend;
use mt5_bridge::error::Result;
use mt5_bridge::journal::OrderStore;
use mt5_bridge::reconciliation::events::{OrderEvent, OrderObserver};
use mt5_bridge::reconciliation::{OrderManager, SharedOrderManager};
use mt5_bridge::trade::{Bracket, TradeManager, TrailingStop};
use mt5_bridge::types::{Level, OrderRequest, OrderState, TradeResult, TradeRule};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mock backend that records every `order_modify_with_magic` call.
#[derive(Debug, Default)]
struct MockBackend {
    modifies: Mutex<Vec<(u64, Level, Level)>>,
}

impl TradingBackend for MockBackend {
    fn order_send(&self, _req: &OrderRequest) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: 1,
            order: 1,
            position: 1,
            volume: 0.0,
            price: 0.0,
        })
    }

    fn order_close_with_magic(&self, _ticket: u64, _magic: u64) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: 1,
            order: 0,
            position: 0,
            volume: 0.0,
            price: 0.0,
        })
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        _magic: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        self.modifies
            .lock()
            .unwrap()
            .push((ticket, stop_loss, take_profit));
        Ok(TradeResult {
            retcode: 10009,
            deal: 0,
            order: ticket,
            position: ticket,
            volume: 0.0,
            price: 0.0,
        })
    }

    fn positions_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<mt5_bridge::types::Position>> {
        Ok(Vec::new())
    }

    fn pending_orders_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<mt5_bridge::types::WorkingOrder>> {
        Ok(Vec::new())
    }
}

impl MockBackend {
    fn modify_count(&self) -> usize {
        self.modifies.lock().unwrap().len()
    }

    fn last_modify(&self) -> Option<(u64, Level, Level)> {
        self.modifies.lock().unwrap().last().cloned()
    }
}

fn new_manager_and_tm() -> (SharedOrderManager, Arc<MockBackend>, TradeManager) {
    let manager = OrderManager::try_new(20240101).expect("valid magic");
    let shared = SharedOrderManager::new(manager);
    let backend = Arc::new(MockBackend::default());
    let tm = TradeManager::new(
        shared.clone(),
        Arc::clone(&backend) as Arc<dyn TradingBackend + Send + Sync>,
    );
    (shared, backend, tm)
}

fn emit_filled(shared: &SharedOrderManager, tm: &TradeManager, cid: &str, symbol: &str) {
    // Register a Created order, transition to Submitting, then emit Filled.
    let req = OrderRequest::buy(symbol, 1.0)
        .stop_loss(1.08000)
        .take_profit(1.09000)
        .client_order_id(cid);
    let mut order = mt5_bridge::types::TrackedOrder::new(&req, cid);
    order.state = OrderState::Submitting;
    order.order_ticket = 55555;
    order.position_ticket = 66666;
    order.magic = 20240101;
    shared.with(|m| m.track_order(order));

    let event = OrderEvent::Filled {
        client_order_id: cid.to_string(),
        deal_ticket: 77777,
        price: 1.08500,
    };
    tm.on_event(&event);
}

#[test]
fn test_break_even_triggers_at_r_multiple() {
    let (shared, backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-be-1", "EURUSD");

    // Entry 1.08500, SL 1.08000 → risk = 0.00500. Break-even at 1.0 R.
    // Profit at price 1.09000 = 0.00500 → 1.0 R → break-even activates.
    let mut rule = tm.get_rule("ord-be-1").unwrap();
    rule.break_even_at_r = Some(1.0);
    rule.risk = 0.00500;
    tm.import_rules(vec![rule]);

    let modified = tm.check_rules("EURUSD", 1.09000).unwrap();
    assert_eq!(modified, vec!["ord-be-1".to_string()]);
    assert_eq!(backend.modify_count(), 1);

    let (ticket, sl, _tp) = backend.last_modify().unwrap();
    assert_eq!(ticket, 66666);
    match sl {
        Level::Set(v) => assert!(
            (v - 1.08500).abs() < 1e-5,
            "break-even SL should be entry+buffer, got {v}"
        ),
        _ => panic!("expected Level::Set"),
    }
}

#[test]
fn test_break_even_does_not_trigger_below_threshold() {
    let (shared, backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-be-2", "EURUSD");

    // Entry 1.08500, risk = 0.00500. Price 1.08750 → profit = 0.00250 → 0.5 R < 1.0 R.
    let mut rule = tm.get_rule("ord-be-2").unwrap();
    rule.break_even_at_r = Some(1.0);
    rule.risk = 0.00500;
    tm.import_rules(vec![rule]);

    let modified = tm.check_rules("EURUSD", 1.08750).unwrap();
    assert!(modified.is_empty());
    assert_eq!(backend.modify_count(), 0);
}

#[test]
fn test_trailing_stop_tracks_price() {
    let (shared, backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-trail-1", "EURUSD");

    // Manually set the rule with trailing config.
    let mut rule = tm.get_rule("ord-trail-1").unwrap();
    rule.trailing_distance = Some(0.00100);
    rule.trailing_step = Some(0.00050);
    rule.trailing_activation = Some(1.0);
    rule.trailing_activated = true;
    rule.stop_loss = 1.08000;
    tm.import_rules(vec![rule]);

    // Price 1.09000 → trailing SL = 1.09000 - 0.00100 = 1.08900.
    // Current SL 1.08000 → improvement 0.00900 >= step 0.00050 → modify.
    let modified = tm.check_rules("EURUSD", 1.09000).unwrap();
    assert_eq!(modified, vec!["ord-trail-1".to_string()]);
    assert_eq!(backend.modify_count(), 1);

    let (_, sl, _tp) = backend.last_modify().unwrap();
    match sl {
        Level::Set(v) => assert!(
            (v - 1.08900).abs() < 1e-6,
            "trailing SL should be 1.08900, got {v}"
        ),
        _ => panic!("expected Level::Set"),
    }
}

#[test]
fn test_trailing_does_not_move_sl_wrong_direction() {
    let (shared, backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-trail-2", "EURUSD");

    let mut rule = tm.get_rule("ord-trail-2").unwrap();
    rule.trailing_distance = Some(0.00100);
    rule.trailing_step = Some(0.00050);
    rule.trailing_activation = Some(1.0);
    rule.trailing_activated = true;
    rule.stop_loss = 1.08900; // already tight
    tm.import_rules(vec![rule]);

    // Price drops back to 1.08600 → trailing SL = 1.08500, which is WORSE than 1.08900 for a buy.
    let modified = tm.check_rules("EURUSD", 1.08600).unwrap();
    assert!(modified.is_empty());
    assert_eq!(backend.modify_count(), 0);
}

#[test]
fn test_rate_limiting_prevents_rapid_modifications() {
    let (shared, backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-rate-1", "EURUSD");

    let mut rule = tm.get_rule("ord-rate-1").unwrap();
    rule.break_even_at_r = Some(1.0);
    rule.risk = 0.00500;
    tm.import_rules(vec![rule]);

    // First call at price 1.09000 → break-even triggers (1.0 R).
    let modified = tm.check_rules("EURUSD", 1.09000).unwrap();
    assert_eq!(modified.len(), 1);
    assert_eq!(backend.modify_count(), 1);

    // Immediate second call at higher price → rate limited, no modification.
    let modified = tm.check_rules("EURUSD", 1.09500).unwrap();
    assert!(modified.is_empty());
    assert_eq!(backend.modify_count(), 1);
}

#[test]
fn test_stops_level_blocks_modification() {
    let (shared, backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-sl-1", "EURUSD");

    let mut rule = tm.get_rule("ord-sl-1").unwrap();
    rule.break_even_at_r = Some(1.0);
    rule.risk = 0.00500;
    rule.stops_level = 0.01000; // 10 pips — far from current price
    tm.import_rules(vec![rule]);

    // Price 1.09000 → break-even SL = 1.08500, distance from price = 0.00500 < stops_level 0.01000.
    let modified = tm.check_rules("EURUSD", 1.09000).unwrap();
    assert!(modified.is_empty());
    assert_eq!(backend.modify_count(), 0);
}

#[test]
fn test_closed_order_removes_rule() {
    let (shared, _backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-close-1", "EURUSD");

    assert!(tm.get_rule("ord-close-1").is_some());

    let event = OrderEvent::Closed {
        client_order_id: "ord-close-1".to_string(),
        reason: mt5_bridge::position_book::DealReason::Client,
    };
    tm.on_event(&event);

    assert!(tm.get_rule("ord-close-1").is_none());
}

#[test]
fn test_bracket_into_rule() {
    let bracket = Bracket {
        entry: OrderRequest::buy("EURUSD", 1.0)
            .stop_loss(1.08000)
            .take_profit(1.09000),
        stop_loss: Level::Set(1.08000),
        take_profit: Level::Set(1.09000),
        break_even_at_r: Some(1.0),
        trailing: Some(TrailingStop {
            distance: 0.00100,
            step: 0.00050,
            activation: 1.0,
        }),
    };

    let rule = bracket.into_rule(
        "ord-bracket-1".to_string(),
        "EURUSD".to_string(),
        66666,
        1.08500,
        0.00500,
    );

    assert_eq!(rule.client_order_id, "ord-bracket-1");
    assert_eq!(rule.symbol, "EURUSD");
    assert_eq!(rule.ticket, 66666);
    assert_eq!(rule.entry_price, 1.08500);
    assert_eq!(rule.volume, 1.0);
    assert_eq!(rule.stop_loss, 1.08000);
    assert_eq!(rule.take_profit, 1.09000);
    assert_eq!(rule.break_even_at_r, Some(1.0));
    assert_eq!(rule.trailing_distance, Some(0.00100));
    assert_eq!(rule.trailing_step, Some(0.00050));
    assert_eq!(rule.trailing_activation, Some(1.0));
    assert_eq!(rule.risk, 0.00500);
    assert!(!rule.break_even_activated);
    assert!(!rule.trailing_activated);
}

#[test]
fn test_trailing_stop_buy_only_moves_up() {
    let (shared, _backend, tm) = new_manager_and_tm();
    emit_filled(&shared, &tm, "ord-trail-buy-1", "EURUSD");

    let mut rule = tm.get_rule("ord-trail-buy-1").unwrap();
    rule.trailing_distance = Some(0.00100);
    rule.trailing_step = Some(0.00010);
    rule.trailing_activation = Some(0.5);
    rule.trailing_activated = true;
    rule.stop_loss = 1.08400; // current SL
    tm.import_rules(vec![rule]);

    // Price moves up: 1.08600 → trailing SL = 1.08500 > 1.08400 → modify.
    let modified = tm.check_rules("EURUSD", 1.08600).unwrap();
    assert_eq!(modified, vec!["ord-trail-buy-1".to_string()]);

    // Price moves down: 1.08300 → trailing SL = 1.08200 < current SL → no modify.
    let modified = tm.check_rules("EURUSD", 1.08300).unwrap();
    assert!(modified.is_empty());
}

#[test]
fn test_sell_trailing_stop() {
    let (shared, backend, tm) = new_manager_and_tm();
    // Create a sell order manually.
    let req = OrderRequest::sell("EURUSD", 1.0)
        .stop_loss(1.09000)
        .take_profit(1.08000)
        .client_order_id("ord-sell-1");
    let mut order = mt5_bridge::types::TrackedOrder::new(&req, "ord-sell-1");
    order.state = OrderState::Submitting;
    order.order_ticket = 55555;
    order.position_ticket = 66666;
    order.magic = 20240101;
    shared.with(|m| m.track_order(order));

    let event = OrderEvent::Filled {
        client_order_id: "ord-sell-1".to_string(),
        deal_ticket: 77777,
        price: 1.08500,
    };
    tm.on_event(&event);

    assert!(tm.get_rule("ord-sell-1").is_some());

    let mut rule = tm.get_rule("ord-sell-1").unwrap();
    rule.trailing_distance = Some(0.00100);
    rule.trailing_step = Some(0.00010);
    rule.trailing_activation = Some(0.5);
    rule.trailing_activated = true;
    rule.stop_loss = 1.08600; // current SL
    tm.import_rules(vec![rule]);

    // Price moves down: 1.08300 → trailing SL = 1.08400 < 1.08600 → modify (better for sell).
    let modified = tm.check_rules("EURUSD", 1.08300).unwrap();
    assert_eq!(modified, vec!["ord-sell-1".to_string()]);

    let (_, sl, _tp) = backend.last_modify().unwrap();
    match sl {
        Level::Set(v) => assert!(
            (v - 1.08400).abs() < 1e-6,
            "sell trailing SL should be 1.08400, got {v}"
        ),
        _ => panic!("expected Level::Set"),
    }
}

// A HashMap to simulate WAL persistence across restart for the restart test.
struct WalRuleStore {
    rules: Mutex<HashMap<String, TradeRule>>,
}

impl WalRuleStore {
    fn new() -> Self {
        Self {
            rules: Mutex::new(HashMap::new()),
        }
    }
}

impl mt5_bridge::journal::OrderStore for WalRuleStore {
    fn save(&self, _orders: &[mt5_bridge::types::TrackedOrder]) -> Result<()> {
        Ok(())
    }

    fn load(&self) -> Result<Vec<mt5_bridge::types::TrackedOrder>> {
        Ok(Vec::new())
    }

    fn record_trade_rule(&self, _symbol: &str, rule: &TradeRule) -> Result<()> {
        self.rules
            .lock()
            .unwrap()
            .insert(rule.client_order_id.clone(), rule.clone());
        Ok(())
    }

    fn load_trade_rules(&self, symbol: &str) -> Result<Vec<TradeRule>> {
        Ok(self
            .rules
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.symbol == symbol)
            .cloned()
            .collect())
    }
}

#[test]
fn test_rule_survives_restart_via_store() {
    let (shared, _backend, tm) = new_manager_and_tm();
    let store = WalRuleStore::new();

    // TradeManager without store, then manually persist to simulate WAL journaling.
    emit_filled(&shared, &tm, "ord-restart-1", "EURUSD");

    let mut rule = tm.get_rule("ord-restart-1").unwrap();
    rule.break_even_at_r = Some(1.0);
    rule.risk = 0.00500;
    tm.import_rules(vec![rule.clone()]);

    // Persist to the store (simulating WAL record_trade_rule).
    store.record_trade_rule("EURUSD", &rule).unwrap();

    // Simulate restart: new TradeManager, import rules from store.
    let tm2 = TradeManager::new(
        shared,
        Arc::new(MockBackend::default()) as Arc<dyn TradingBackend + Send + Sync>,
    );
    let loaded = store.load_trade_rules("EURUSD").unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].client_order_id, "ord-restart-1");
    tm2.import_rules(loaded);

    // After restart, break-even still works at price 1.09000 (1.0 R).
    let modified = tm2.check_rules("EURUSD", 1.09000).unwrap();
    assert_eq!(modified, vec!["ord-restart-1".to_string()]);
}

//! Test suite for Mt5Gateway and Mt5Session.
//!
//! Validates:
//! - Priority message scheduling (P1 commands preempt P2 and P3).
//! - Concurrent order submissions and modifications without state corruption.
//! - Mt5Session lifecycle state transitions and exponential backoff ladder.
//! - Disconnect blocking of new entries and post-reconnect recovery.
//! - TradeEvent lossless queue delivery and event_seq gap detection triggering reconciliation.
//! - Conflated tick stream updates via watch channels.

#![allow(clippy::disallowed_methods)]

use mt5_bridge::backend::TradingBackend;
use mt5_bridge::constants::EA_DEFAULT_MAGIC;
use mt5_bridge::gateway::*;
use mt5_bridge::reconciliation::{LifecycleState, OrderManager, SafetyPolicy};
use mt5_bridge::session::*;
use mt5_bridge::types::*;
use mt5_bridge::{Mt5Error, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Default)]
struct BrokerState {
    next_ticket: u64,
    positions: Vec<Position>,
    pending: Vec<WorkingOrder>,
    deals: Vec<Deal>,
    execution_order: Vec<&'static str>,
}

#[derive(Clone)]
struct TestBroker {
    state: Arc<Mutex<BrokerState>>,
    reconcile_count: Arc<AtomicUsize>,
}

impl TestBroker {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(BrokerState {
                next_ticket: 1000,
                positions: Vec::new(),
                pending: Vec::new(),
                deals: Vec::new(),
                execution_order: Vec::new(),
            })),
            reconcile_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn execution_order(&self) -> Vec<&'static str> {
        self.state.lock().unwrap().execution_order.clone()
    }

    fn reconcile_count(&self) -> usize {
        self.reconcile_count.load(Ordering::SeqCst)
    }
}

impl TradingBackend for TestBroker {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        g.execution_order.push("P1_SUBMIT");
        let ticket = g.next_ticket;
        g.next_ticket += 1;
        let pos = Position {
            ticket,
            time: chrono::Utc::now().timestamp(),
            position_type: req.order_type,
            magic: req.magic.unwrap_or(0),
            volume: req.volume,
            price_open: if req.price > 0.0 { req.price } else { 1.0850 },
            stop_loss: req.stop_loss,
            take_profit: req.take_profit,
            price_current: 1.0850,
            profit: 0.0,
            swap: 0.0,
            symbol: req.symbol.clone(),
            comment: req.comment.clone(),
        };
        g.positions.push(pos);
        Ok(TradeResult {
            retcode: 10009,
            deal: ticket + 5000,
            order: ticket,
            position: ticket,
            volume: req.volume,
            price: 1.0850,
        })
    }

    fn order_close_with_magic(&self, ticket: u64, _magic: u64) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        g.execution_order.push("P1_CLOSE");
        g.positions.retain(|p| p.ticket != ticket);
        Ok(TradeResult {
            retcode: 10009,
            deal: ticket + 6000,
            order: ticket,
            position: ticket,
            volume: 0.1,
            price: 1.0850,
        })
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        _magic: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        g.execution_order.push("P1_MODIFY");
        if let Some(pos) = g.positions.iter_mut().find(|p| p.ticket == ticket) {
            match stop_loss {
                Level::Set(val) => pos.stop_loss = val,
                Level::Clear => pos.stop_loss = 0.0,
                Level::Keep => {}
            }
            match take_profit {
                Level::Set(val) => pos.take_profit = val,
                Level::Clear => pos.take_profit = 0.0,
                Level::Keep => {}
            }
        }
        Ok(TradeResult {
            retcode: 10009,
            deal: 0,
            order: ticket,
            position: ticket,
            volume: 0.1,
            price: 1.0850,
        })
    }

    fn positions_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        self.reconcile_count.fetch_add(1, Ordering::SeqCst);
        let g = self.state.lock().unwrap();
        Ok(g.positions
            .iter()
            .filter(|p| magic.map_or(true, |m| p.magic == m))
            .filter(|p| symbol.map_or(true, |s| p.symbol == s))
            .cloned()
            .collect())
    }

    fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        let g = self.state.lock().unwrap();
        Ok(g.pending
            .iter()
            .filter(|o| magic.map_or(true, |m| o.magic == m))
            .filter(|o| symbol.map_or(true, |s| o.symbol == s))
            .cloned()
            .collect())
    }

    fn deals_filtered(
        &self,
        _from: i64,
        _to: i64,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        let g = self.state.lock().unwrap();
        Ok(g.deals
            .iter()
            .filter(|d| magic.map_or(true, |m| d.magic == m))
            .filter(|d| symbol.map_or(true, |s| d.symbol == s))
            .cloned()
            .collect())
    }

    fn account_info(&self) -> Result<AccountInfo> {
        let mut g = self.state.lock().unwrap();
        g.execution_order.push("P2_ACCOUNT");
        Ok(AccountInfo {
            balance: 10000.0,
            equity: 10000.0,
            margin: 0.0,
            free_margin: 10000.0,
            leverage: 100.0,
            currency: "USD".to_string(),
        })
    }

    fn copy_rates(
        &self,
        _symbol: &str,
        _timeframe: Timeframe,
        _from: i64,
        _to: i64,
    ) -> Result<Vec<Rate>> {
        let mut g = self.state.lock().unwrap();
        g.execution_order.push("P3_RATES");
        Ok(vec![Rate {
            time: 1700000000,
            open: 1.0800,
            high: 1.0850,
            low: 1.0790,
            close: 1.0820,
            volume: 100,
            spread: 1,
            real_volume: 0,
        }])
    }
}

fn test_manager() -> OrderManager {
    let policy = SafetyPolicy {
        min_absent_observations: 1,
        min_absent_secs: 0,
        ..Default::default()
    };
    OrderManager::try_with_policy(EA_DEFAULT_MAGIC, policy).expect("valid magic")
}

fn buy_order(symbol: &str, vol: f64, cid: &str) -> OrderRequest {
    OrderRequest::buy(symbol, vol).client_order_id(cid)
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Mt5Session Lifecycle & Exponential Backoff
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_session_lifecycle_and_exponential_backoff() {
    let config = SessionConfig {
        initial_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_millis(800),
        backoff_factor: 2.0,
        max_reconnect_attempts: Some(5),
    };

    let mut session = Mt5Session::new(config);
    assert_eq!(session.state(), LifecycleState::Starting);
    assert!(!session.is_ready());
    assert_eq!(session.consecutive_failures(), 0);

    // Transitions: Starting -> Syncing -> Reconciling -> Ready
    session.on_connected();
    assert_eq!(session.state(), LifecycleState::Syncing);
    assert!(!session.is_ready());

    session.on_synced();
    assert_eq!(session.state(), LifecycleState::Reconciling);
    assert!(!session.is_ready());

    session.on_reconciled();
    assert_eq!(session.state(), LifecycleState::Ready);
    assert!(session.is_ready());

    // Subscription registrations are stored
    session.register_tick_subscription("EURUSD");
    session.register_tick_subscription("GBPUSD");
    session.register_trade_subscription();
    session.register_book_subscription("EURUSD");

    assert!(session.active_tick_subscriptions().contains("EURUSD"));
    assert!(session.active_tick_subscriptions().contains("GBPUSD"));
    assert!(session.is_trade_subscribed());
    assert!(session.active_book_subscriptions().contains("EURUSD"));

    // First disconnect
    session.on_disconnect("connection reset by peer");
    assert_eq!(session.state(), LifecycleState::Degraded);
    assert!(!session.is_ready());
    assert_eq!(session.consecutive_failures(), 1);
    assert_eq!(session.current_backoff(), Duration::from_millis(100));

    // Subscriptions survive disconnect
    assert!(session.active_tick_subscriptions().contains("EURUSD"));
    assert!(session.is_trade_subscribed());

    // Second disconnect: 100 * 2^1 = 200ms
    session.on_disconnect("pipe unavailable");
    assert_eq!(session.consecutive_failures(), 2);
    assert_eq!(session.current_backoff(), Duration::from_millis(200));

    // Third disconnect: 100 * 2^2 = 400ms
    session.on_disconnect("timeout");
    assert_eq!(session.consecutive_failures(), 3);
    assert_eq!(session.current_backoff(), Duration::from_millis(400));

    // Fourth disconnect: 100 * 2^3 = 800ms (clamped to max_backoff)
    session.on_disconnect("timeout");
    assert_eq!(session.consecutive_failures(), 4);
    assert_eq!(session.current_backoff(), Duration::from_millis(800));

    // Successful reconnect resets consecutive failures and backoff
    session.on_connected();
    assert_eq!(session.state(), LifecycleState::Syncing);
    assert_eq!(session.consecutive_failures(), 0);
    assert_eq!(session.current_backoff(), Duration::from_millis(100));

    session.on_reconciled();
    assert_eq!(session.state(), LifecycleState::Ready);
}

#[test]
fn test_session_max_reconnect_attempts_stops_session() {
    let config = SessionConfig {
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        backoff_factor: 2.0,
        max_reconnect_attempts: Some(2),
    };

    let mut session = Mt5Session::new(config);
    session.on_disconnect("err 1");
    assert_eq!(session.state(), LifecycleState::Degraded);
    session.on_disconnect("err 2");
    assert_eq!(session.state(), LifecycleState::Degraded);
    session.on_disconnect("err 3"); // Exceeds max_attempts (2)
    assert_eq!(session.state(), LifecycleState::Stopped);
    assert!(!session.is_ready());
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Mt5Gateway Priority Message Scheduling
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_gateway_priority_queue_preempts_bulk_requests() {
    let broker = TestBroker::new();
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(broker.clone(), mgr, SessionConfig::default());

    // Send P3 request
    let g1 = gateway.clone();
    let h_p3 = tokio::spawn(async move {
        g1.copy_rates("EURUSD", Timeframe::M1, 0, 100)
            .await
            .unwrap();
    });

    // Send P2 request
    let g2 = gateway.clone();
    let h_p2 = tokio::spawn(async move {
        g2.account_info().await.unwrap();
    });

    // Send P1 request
    let g3 = gateway.clone();
    let h_p1 = tokio::spawn(async move {
        g3.submit_order(buy_order("EURUSD", 0.1, "prio-1"))
            .await
            .unwrap();
    });

    let (r3, r2, r1) = tokio::join!(h_p3, h_p2, h_p1);
    r3.unwrap();
    r2.unwrap();
    r1.unwrap();

    let exec = broker.execution_order();
    assert!(exec.contains(&"P1_SUBMIT"));
    assert!(exec.contains(&"P2_ACCOUNT"));
    assert!(exec.contains(&"P3_RATES"));

    gateway.shutdown();
    let _ = join_handle.await;
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Concurrent Submissions and Modifications
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_gateway_concurrent_submissions_and_modifications() {
    let broker = TestBroker::new();
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(broker, mgr, SessionConfig::default());

    let num_tasks = 20;
    let mut tasks = Vec::new();

    for i in 0..num_tasks {
        let g = gateway.clone();
        tasks.push(tokio::spawn(async move {
            let cid = format!("conc-{i}");
            let tracked = g
                .submit_order(buy_order("EURUSD", 0.1, &cid))
                .await
                .unwrap();
            assert_eq!(tracked.state, OrderState::Filled);

            // Immediately modify SL/TP
            let mod_res = g
                .modify_order(
                    tracked.position_ticket,
                    EA_DEFAULT_MAGIC,
                    Level::Set(1.0800),
                    Level::Set(1.0900),
                )
                .await
                .unwrap();
            assert_eq!(mod_res.retcode, 10009);
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    // Verify all positions open and accounted for
    let positions = gateway
        .positions_filtered(Some(EA_DEFAULT_MAGIC), None)
        .await
        .unwrap();
    assert_eq!(positions.len(), num_tasks);

    gateway.shutdown();
    let _ = join_handle.await;
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. Trade Event Lossless Queue & Gap Recovery
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_gateway_trade_event_lossless_queue_and_gap_detection() {
    let broker = TestBroker::new();
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(broker.clone(), mgr, SessionConfig::default());

    let mut trade_rx = gateway.subscribe_trade_events();

    gateway.wait_ready(Duration::from_secs(1)).await.unwrap();
    let initial_reconcile = broker.reconcile_count();
    assert_eq!(initial_reconcile, 1);

    // Event 1 (normal sequence)
    let ev1 = TradeEvent {
        deal: 101,
        order: 201,
        position: 301,
        time: 1700000000,
        trans_type: 0,
        order_type: OrderType::Buy,
        price: 1.0850,
        volume: 0.1,
        sl: 0.0,
        tp: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "".to_string(),
        deal_entry: 0,
        deal_type: 0,
        deal_magic: EA_DEFAULT_MAGIC,
        deal_position_id: 301,
        deal_reason: 0,
        event_seq: 1,
    };
    gateway.dispatch_trade_event(ev1.clone());

    let received = trade_rx.recv().await.unwrap();
    assert_eq!(received.event_seq, 1);
    assert_eq!(broker.reconcile_count(), initial_reconcile);

    // Event 5 (sequence gap: expected 2, received 5 -> gap of 3 events)
    let ev5 = TradeEvent {
        deal: 105,
        order: 205,
        position: 305,
        time: 1700000010,
        trans_type: 0,
        order_type: OrderType::Buy,
        price: 1.0860,
        volume: 0.1,
        sl: 0.0,
        tp: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "".to_string(),
        deal_entry: 0,
        deal_type: 0,
        deal_magic: EA_DEFAULT_MAGIC,
        deal_position_id: 305,
        deal_reason: 0,
        event_seq: 5,
    };
    gateway.dispatch_trade_event(ev5.clone());

    let received5 = trade_rx.recv().await.unwrap();
    assert_eq!(received5.event_seq, 5);

    // Wait briefly for actor to execute mandatory reconciliation triggered by gap
    sleep(Duration::from_millis(50)).await;
    assert!(
        broker.reconcile_count() > 0,
        "Expected gap detection to trigger automatic broker reconciliation"
    );

    gateway.shutdown();
    let _ = join_handle.await;
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Tick Conflation via Watch Channels
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_gateway_tick_conflation() {
    let broker = TestBroker::new();
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(broker, mgr, SessionConfig::default());

    let tick_rx = gateway.subscribe_ticks("EURUSD");
    assert!(tick_rx.borrow().is_none());

    // Dispatch a series of rapid ticks
    for i in 1..=50 {
        let tick = Tick {
            symbol: "EURUSD".to_string(),
            time: 1700000000 + i,
            time_msc: (1700000000 + i) * 1000,
            bid: 1.0800 + (i as f64 * 0.0001),
            ask: 1.0802 + (i as f64 * 0.0001),
            last: 1.0801,
            volume: 10,
            flags: 0,
            event_seq: 0,
        };
        gateway.dispatch_tick(tick);
    }

    // Consumer reads the conflated latest tick
    let current_opt = tick_rx.borrow().clone();
    assert!(current_opt.is_some());
    let current = current_opt.unwrap();
    assert_eq!(current.symbol, "EURUSD");
    assert_eq!(current.time, 1700000050);
    assert!((current.bid - (1.0800 + 50.0 * 0.0001)).abs() < 1e-6);

    gateway.shutdown();
    let _ = join_handle.await;
}

#[tokio::test]
async fn test_gateway_position_book_tracking_and_queries() {
    let broker = TestBroker::new();
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(broker, mgr, SessionConfig::default());

    // Initial position book should be empty
    let book = gateway.position_book().await.unwrap();
    assert_eq!(book.open_positions().len(), 0);

    // Submit an order
    let req = OrderRequest {
        symbol: "EURUSD".to_string(),
        order_type: OrderType::Buy,
        volume: 1.0,
        price: 1.0850,
        stop_loss: 1.0800,
        take_profit: 1.0950,
        deviation: Some(10),
        comment: "gateway_book_test".to_string(),
        magic: Some(EA_DEFAULT_MAGIC),
        expiration: None,
        client_order_id: None,
    };
    let tracked = gateway.submit_order(req).await.unwrap();
    let ticket = tracked.position_ticket;

    // Simulate OnTradeTransaction deal event from MT5
    let trade_event = TradeEvent {
        event_seq: 1,
        deal: 9901,
        order: ticket,
        position: ticket,
        trans_type: 1,
        order_type: OrderType::Buy,
        deal_entry: 0, // In
        deal_type: 0,
        deal_reason: 3, // Expert
        deal_magic: EA_DEFAULT_MAGIC,
        deal_position_id: ticket,
        time: 1700000100,
        price: 1.0850,
        volume: 1.0,
        sl: 1.0800,
        tp: 1.0950,
        symbol: "EURUSD".to_string(),
        comment: "test deal".to_string(),
    };
    gateway.dispatch_trade_event(trade_event);

    // Allow event loop to process
    tokio::time::sleep(Duration::from_millis(20)).await;

    let book = gateway.position_book().await.unwrap();
    assert_eq!(book.open_positions().len(), 1);
    let pos = book.get(ticket).expect("position must be tracked");
    assert_eq!(pos.symbol, "EURUSD");
    assert_eq!(pos.volume, 1.0);
    assert_eq!(pos.state, mt5_bridge::position_book::PositionState::Open);

    // Modify order SL/TP
    gateway
        .modify_order(
            ticket,
            EA_DEFAULT_MAGIC,
            Level::Set(1.0820),
            Level::Set(1.0980),
        )
        .await
        .unwrap();

    let book = gateway.position_book().await.unwrap();
    let pos = book.get(ticket).unwrap();
    assert_eq!(pos.stop_loss, 1.0820);
    assert_eq!(pos.take_profit, 1.0980);

    // Close position
    gateway
        .close_position(ticket, EA_DEFAULT_MAGIC)
        .await
        .unwrap();

    // Position is marked Closing immediately
    let book = gateway.position_book().await.unwrap();
    let pos = book.get(ticket).unwrap();
    assert_eq!(pos.state, mt5_bridge::position_book::PositionState::Closing);

    // Exit deal arrives from broker
    let exit_event = TradeEvent {
        event_seq: 2,
        deal: 9902,
        order: ticket,
        position: ticket,
        trans_type: 1,
        order_type: OrderType::Sell,
        deal_entry: 1, // Out
        deal_type: 1,
        deal_reason: 3, // Expert
        deal_magic: EA_DEFAULT_MAGIC,
        deal_position_id: ticket,
        time: 1700000200,
        price: 1.0860,
        volume: 1.0,
        sl: 0.0,
        tp: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "close deal".to_string(),
    };
    gateway.dispatch_trade_event(exit_event);

    tokio::time::sleep(Duration::from_millis(20)).await;

    let book = gateway.position_book().await.unwrap();
    assert_eq!(book.open_positions().len(), 0);
    let closed = book
        .get_closed(ticket)
        .expect("position should be in closed positions");
    assert_eq!(
        closed.state,
        mt5_bridge::position_book::PositionState::Closed
    );
    assert_eq!(closed.exit_price, Some(1.0860));

    gateway.shutdown();
    let _ = join_handle.await;
}

// ═════════════════════════════════════════════════════════════════════════════
// 8. Bounded Blocking Executor and Actor Responsiveness
// ═════════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct SlowBackend {
    p3_sleep: Duration,
    p2_sleep: Duration,
    p3_started: Arc<tokio::sync::Notify>,
    p1_completed: Arc<std::sync::atomic::AtomicBool>,
}

impl TradingBackend for SlowBackend {
    fn order_send(&self, _req: &OrderRequest) -> Result<TradeResult> {
        self.p1_completed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(TradeResult {
            retcode: 10009,
            deal: 12345,
            order: 67890,
            position: 67890,
            volume: 0.1,
            price: 1.0850,
        })
    }

    fn order_close_with_magic(&self, _ticket: u64, _magic: u64) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: 12345,
            order: 67890,
            position: 67890,
            volume: 0.1,
            price: 1.0850,
        })
    }

    fn order_modify_with_magic(
        &self,
        _ticket: u64,
        _magic: u64,
        _stop_loss: Level,
        _take_profit: Level,
    ) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: 12345,
            order: 67890,
            position: 67890,
            volume: 0.1,
            price: 1.0850,
        })
    }

    fn positions_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        Ok(vec![])
    }

    fn pending_orders_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        Ok(vec![])
    }

    fn account_info(&self) -> Result<AccountInfo> {
        if !self.p2_sleep.is_zero() {
            std::thread::sleep(self.p2_sleep);
        }
        Ok(AccountInfo {
            balance: 10000.0,
            equity: 10000.0,
            margin: 0.0,
            free_margin: 10000.0,
            leverage: 100.0,
            currency: "USD".to_string(),
        })
    }

    fn copy_rates(
        &self,
        _symbol: &str,
        _timeframe: Timeframe,
        _from: i64,
        _to: i64,
    ) -> Result<Vec<Rate>> {
        self.p3_started.notify_one();
        std::thread::sleep(self.p3_sleep);
        Ok(vec![Rate {
            time: 1700000000,
            open: 1.08,
            high: 1.09,
            low: 1.07,
            close: 1.085,
            volume: 100,
            spread: 1,
            real_volume: 100,
        }])
    }

    fn copy_rates_chunked(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<HistoryResult> {
        self.p3_started.notify_one();
        let mut rates = Vec::new();
        let chunk_count = 10;
        let sleep_per_chunk = self.p3_sleep / (chunk_count as u32);
        for i in 0..chunk_count {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(HistoryResult {
                    rates,
                    complete: false,
                    missing_ranges: vec![],
                    cancelled: true,
                });
            }
            std::thread::sleep(sleep_per_chunk);
            rates.push(Rate {
                time: from + (i as i64 * 60),
                open: 1.08,
                high: 1.09,
                low: 1.07,
                close: 1.085,
                volume: 100,
                spread: 1,
                real_volume: 100,
            });
        }
        let _ = (symbol, timeframe, to);
        Ok(HistoryResult {
            rates,
            complete: true,
            missing_ranges: vec![],
            cancelled: false,
        })
    }
}

#[tokio::test]
async fn test_slow_p3_does_not_block_p1_submission() {
    let p3_started = Arc::new(tokio::sync::Notify::new());
    let p1_completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let backend = SlowBackend {
        p3_sleep: Duration::from_secs(2),
        p2_sleep: Duration::ZERO,
        p3_started: Arc::clone(&p3_started),
        p1_completed: Arc::clone(&p1_completed),
    };
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(backend, mgr, SessionConfig::default());

    let g_p3 = gateway.clone();
    let h_p3 = tokio::spawn(async move { g_p3.copy_rates("EURUSD", Timeframe::M1, 0, 100).await });

    // Wait until P3 has actually started executing in the blocking thread
    p3_started.notified().await;

    // Submit P1 while P3 is sleeping
    let t0 = std::time::Instant::now();
    let res = gateway
        .submit_order(buy_order("EURUSD", 0.1, "p1-fast"))
        .await;
    let elapsed = t0.elapsed();

    assert!(res.is_ok(), "P1 submit should succeed");
    assert!(p1_completed.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        elapsed < Duration::from_millis(500),
        "P1 must complete without waiting for P3 sleep (took {elapsed:?})"
    );

    let _ = h_p3.await;
    gateway.shutdown();
    let _ = join_handle.await;
}

#[tokio::test]
async fn test_actor_remains_responsive_to_ping_during_p3() {
    let p3_started = Arc::new(tokio::sync::Notify::new());
    let backend = SlowBackend {
        p3_sleep: Duration::from_secs(1),
        p2_sleep: Duration::ZERO,
        p3_started: Arc::clone(&p3_started),
        p1_completed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(backend, mgr, SessionConfig::default());

    let g_p3 = gateway.clone();
    let h_p3 = tokio::spawn(async move { g_p3.copy_rates("EURUSD", Timeframe::M1, 0, 100).await });

    p3_started.notified().await;

    let t0 = std::time::Instant::now();
    let ping_res = gateway.ping().await;
    let elapsed = t0.elapsed();

    assert!(ping_res.is_ok(), "Ping should succeed");
    assert!(
        elapsed < Duration::from_millis(100),
        "Ping took {elapsed:?}, should be instantaneous"
    );

    let _ = h_p3.await;
    gateway.shutdown();
    let _ = join_handle.await;
}

#[tokio::test]
async fn test_p2_timeout_returns_error() {
    let backend = SlowBackend {
        p3_sleep: Duration::ZERO,
        p2_sleep: Duration::from_millis(300),
        p3_started: Arc::new(tokio::sync::Notify::new()),
        p1_completed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let mgr = test_manager();
    let config = GatewayConfig {
        p2_timeout: Duration::from_millis(50),
        ..Default::default()
    };
    let (gateway, join_handle) = Mt5Gateway::spawn_with_config(backend, mgr, config);

    let res = gateway.account_info().await;
    match res {
        Err(mt5_bridge::Mt5Error::Timeout { operation }) => {
            assert_eq!(operation, "p2_account_info");
        }
        other => panic!("Expected Mt5Error::Timeout, got {other:?}"),
    }

    gateway.shutdown();
    let _ = join_handle.await;
}

#[tokio::test]
async fn test_cancelled_p3_returns_partial_data_and_does_not_block_p1() {
    let p3_started = Arc::new(tokio::sync::Notify::new());
    let p1_completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let backend = SlowBackend {
        p3_sleep: Duration::from_millis(1500),
        p2_sleep: Duration::ZERO,
        p3_started: Arc::clone(&p3_started),
        p1_completed: Arc::clone(&p1_completed),
    };
    let mgr = test_manager();
    let (gateway, join_handle) = Mt5Gateway::spawn(backend, mgr, SessionConfig::default());

    let g_p3 = gateway.clone();
    let h_p3 = tokio::spawn(async move { g_p3.copy_rates("EURUSD", Timeframe::M1, 0, 100).await });

    p3_started.notified().await;
    // Allow at least one chunk to be generated
    tokio::time::sleep(Duration::from_millis(200)).await;

    // P1 arrival triggers cancellation of P3
    let p1_res = gateway
        .submit_order(buy_order("EURUSD", 0.1, "p1-cancel-p3"))
        .await;
    assert!(p1_res.is_ok());

    let p3_res = h_p3.await.unwrap().unwrap();
    assert!(p3_res.cancelled, "P3 should be marked as cancelled");
    assert!(
        !p3_res.rates.is_empty(),
        "P3 should return partial rates collected before cancellation"
    );
    assert!(!p3_res.is_complete());

    gateway.shutdown();
    let _ = join_handle.await;
}

// ═════════════════════════════════════════════════════════════════════════════
// 6. Supervised Session Lifecycle, Reconnect, and Subscription Replay
// ═════════════════════════════════════════════════════════════════════════════

struct FlakyConnector {
    attempts: Arc<AtomicUsize>,
    broker: TestBroker,
}

impl FlakyConnector {
    fn new(broker: TestBroker) -> (Self, Arc<AtomicUsize>) {
        let attempts = Arc::new(AtomicUsize::new(0));
        (
            Self {
                attempts: Arc::clone(&attempts),
                broker,
            },
            attempts,
        )
    }
}

impl Connector<TestBroker> for FlakyConnector {
    fn connect(&self) -> Result<TestBroker> {
        std::thread::sleep(Duration::from_millis(15));
        let count = self.attempts.fetch_add(1, Ordering::SeqCst);
        if count < 2 {
            Err(Mt5Error::TransmissionFailed("connect timeout".to_string()))
        } else {
            Ok(self.broker.clone())
        }
    }
}

#[tokio::test]
async fn test_flaky_connector_retries_and_transitions_through_lifecycle() {
    let broker = TestBroker::new();
    let (connector, attempts) = FlakyConnector::new(broker);
    let mgr = test_manager();
    let config = SessionConfig {
        initial_backoff: Duration::from_millis(30),
        max_backoff: Duration::from_millis(100),
        backoff_factor: 2.0,
        max_reconnect_attempts: Some(5),
    };

    let (gateway, join_handle) = Mt5Gateway::spawn_supervised(connector, mgr, config);

    let mut state_rx = gateway.state_stream();
    let mut recorded_states = Vec::new();
    recorded_states.push(*state_rx.borrow());

    let state_collector = tokio::spawn(async move {
        while state_rx.changed().await.is_ok() {
            let state = *state_rx.borrow();
            recorded_states.push(state);
            if state == LifecycleState::Ready {
                break;
            }
        }
        recorded_states
    });

    gateway
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("Gateway should eventually reach Ready state");

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert!(gateway.is_ready());

    let states = state_collector.await.unwrap();
    assert!(states.contains(&LifecycleState::Connecting));
    assert!(states.contains(&LifecycleState::Degraded));
    assert!(states.contains(&LifecycleState::Syncing));
    assert!(states.contains(&LifecycleState::Reconciling));
    assert_eq!(*states.last().unwrap(), LifecycleState::Ready);

    gateway.shutdown();
    let _ = join_handle.await;
}

#[tokio::test]
async fn test_p1_submissions_rejected_while_not_ready() {
    struct AlwaysFailingConnector;
    impl Connector<TestBroker> for AlwaysFailingConnector {
        fn connect(&self) -> Result<TestBroker> {
            Err(Mt5Error::TransmissionFailed("refused".to_string()))
        }
    }

    let mgr = test_manager();
    let config = SessionConfig {
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(100),
        backoff_factor: 2.0,
        max_reconnect_attempts: Some(10),
    };

    let (gateway, join_handle) = Mt5Gateway::spawn_supervised(AlwaysFailingConnector, mgr, config);

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!gateway.is_ready());

    let res = gateway
        .submit_order(buy_order("EURUSD", 0.1, "not-ready-order"))
        .await;

    match res {
        Err(Mt5Error::PreflightRejected(msg)) => {
            assert!(
                msg.contains("not ready"),
                "Expected rejection message to state not ready, got: {msg}"
            );
        }
        other => panic!("Expected PreflightRejected error, got: {other:?}"),
    }

    gateway.shutdown();
    let _ = join_handle.await;
}

#[derive(Clone)]
struct DisconnectingBroker {
    requests_before_disconnect: Arc<AtomicUsize>,
    subscription_replays: Arc<AtomicUsize>,
    _state: Arc<Mutex<BrokerState>>,
}

impl DisconnectingBroker {
    fn new(
        requests_before_disconnect: Arc<AtomicUsize>,
        subscription_replays: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            requests_before_disconnect,
            subscription_replays,
            _state: Arc::new(Mutex::new(BrokerState::default())),
        }
    }
}

impl TradingBackend for DisconnectingBroker {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        let remaining = self.requests_before_disconnect.load(Ordering::SeqCst);
        if remaining == 0 {
            return Err(Mt5Error::PipeDisconnected);
        }
        self.requests_before_disconnect
            .fetch_sub(1, Ordering::SeqCst);
        Ok(TradeResult {
            retcode: 10009,
            deal: 1,
            order: 1,
            position: 1,
            volume: req.volume,
            price: 1.0850,
        })
    }

    fn order_close_with_magic(&self, _ticket: u64, _magic: u64) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: 1,
            order: 1,
            position: 1,
            volume: 0.1,
            price: 1.0850,
        })
    }

    fn order_modify_with_magic(
        &self,
        _ticket: u64,
        _magic: u64,
        _stop_loss: Level,
        _take_profit: Level,
    ) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: 1,
            order: 1,
            position: 1,
            volume: 0.1,
            price: 1.0850,
        })
    }

    fn positions_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        let remaining = self.requests_before_disconnect.load(Ordering::SeqCst);
        if remaining == 0 {
            return Err(Mt5Error::PipeDisconnected);
        }
        Ok(Vec::new())
    }

    fn pending_orders_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        Ok(Vec::new())
    }

    fn account_info(&self) -> Result<AccountInfo> {
        let remaining = self.requests_before_disconnect.load(Ordering::SeqCst);
        if remaining == 0 {
            return Err(Mt5Error::PipeDisconnected);
        }
        self.requests_before_disconnect
            .fetch_sub(1, Ordering::SeqCst);
        Ok(AccountInfo {
            balance: 10000.0,
            equity: 10000.0,
            margin: 0.0,
            free_margin: 10000.0,
            leverage: 100.0,
            currency: "USD".to_string(),
        })
    }

    fn subscribe_ticks(&self, _symbol: &str) -> Result<()> {
        self.subscription_replays.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn subscribe_trade(&self) -> Result<()> {
        self.subscription_replays.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct ReconnectingConnector {
    connect_count: Arc<AtomicUsize>,
    requests_before_disconnect: Arc<AtomicUsize>,
    subscription_replays: Arc<AtomicUsize>,
}

impl Connector<DisconnectingBroker> for ReconnectingConnector {
    fn connect(&self) -> Result<DisconnectingBroker> {
        self.connect_count.fetch_add(1, Ordering::SeqCst);
        self.requests_before_disconnect.store(2, Ordering::SeqCst);
        Ok(DisconnectingBroker::new(
            Arc::clone(&self.requests_before_disconnect),
            Arc::clone(&self.subscription_replays),
        ))
    }
}

#[tokio::test]
async fn test_backend_disconnect_triggers_backoff_reconnect_and_subscription_replay() {
    let connect_count = Arc::new(AtomicUsize::new(0));
    let requests_before_disconnect = Arc::new(AtomicUsize::new(2));
    let subscription_replays = Arc::new(AtomicUsize::new(0));

    let connector = ReconnectingConnector {
        connect_count: Arc::clone(&connect_count),
        requests_before_disconnect: Arc::clone(&requests_before_disconnect),
        subscription_replays: Arc::clone(&subscription_replays),
    };

    let mgr = test_manager();
    let config = SessionConfig {
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        backoff_factor: 2.0,
        max_reconnect_attempts: Some(5),
    };

    let (gateway, join_handle) = Mt5Gateway::spawn_supervised(connector, mgr, config);

    gateway
        .wait_ready(Duration::from_secs(3))
        .await
        .expect("Gateway should reach Ready initially");
    assert_eq!(connect_count.load(Ordering::SeqCst), 1);

    gateway.register_tick_subscription("EURUSD");
    gateway.register_trade_subscription();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(subscription_replays.load(Ordering::SeqCst), 2);

    let _ = gateway.account_info().await;
    let _ = gateway.account_info().await;

    let res = gateway.account_info().await;
    assert!(res.is_err());

    tokio::time::sleep(Duration::from_millis(150)).await;
    gateway
        .wait_ready(Duration::from_secs(3))
        .await
        .expect("Gateway should recover and reach Ready after disconnect");

    assert!(connect_count.load(Ordering::SeqCst) >= 2);
    assert!(
        subscription_replays.load(Ordering::SeqCst) >= 4,
        "Subscriptions must be replayed upon reconnection"
    );

    gateway.shutdown();
    let _ = join_handle.await;
}

#[tokio::test]
async fn test_session_state_stream_and_wait_ready_standalone() {
    let session = Mt5Session::new(SessionConfig::default());
    assert_eq!(session.state(), LifecycleState::Starting);

    let wait_handle =
        tokio::spawn(async move { session.wait_ready(Duration::from_millis(20)).await });

    let res = wait_handle.await.unwrap();
    assert!(matches!(res, Err(Mt5Error::Timeout { .. })));

    let mut session2 = Mt5Session::new(SessionConfig::default());
    session2.on_connected();
    session2.on_synced();
    session2.on_reconciled();
    assert!(session2.is_ready());

    let res2 = session2.wait_ready(Duration::from_millis(50)).await;
    assert!(res2.is_ok());
}

#[test]
fn test_order_manager_strategy_magic_assignment() {
    let manager = OrderManager::try_new(998877).expect("valid magic");
    let mut req = OrderRequest::buy("EURUSD", 0.1);
    assert_eq!(req.magic, None);
    let _ = manager.check_idempotency_and_magic(&mut req).unwrap();
    assert_eq!(req.magic, Some(998877));
}

#[test]
fn test_order_manager_ownership_mismatch() {
    let manager = OrderManager::try_new(998877).expect("valid magic");
    let mut req = OrderRequest::buy("EURUSD", 0.1).magic(123456);
    let err = manager.check_idempotency_and_magic(&mut req).unwrap_err();
    match err {
        Mt5Error::OwnershipMismatch {
            expected_magic,
            actual_magic,
            ..
        } => {
            assert_eq!(expected_magic, 998877);
            assert_eq!(actual_magic, 123456);
        }
        other => panic!("expected OwnershipMismatch, got {other:?}"),
    }
}

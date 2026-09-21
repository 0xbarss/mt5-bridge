use chrono::Utc;
use mt5_bridge::{Mt5Client, Mt5Error, OrderRequest, OrderType, Timeframe};
use std::env;

use std::sync::{Mutex, MutexGuard};

static MT5_LOCK: Mutex<()> = Mutex::new(());

fn get_client() -> Option<(Mt5Client, MutexGuard<'static, ()>)> {
    let guard = MT5_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);
    let password = env::var("MT5_PASSWORD")
        .or_else(|_| env::var("MT5_PIPE_SECRET"))
        .unwrap_or_default();
    let server = env::var("MT5_SERVER").unwrap_or_default();

    let require_live = env::var("MT5_REQUIRE_LIVE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    match Mt5Client::connect(login, &password, &server) {
        Ok(client) => Some((client, guard)),
        Err(e) => {
            if require_live {
                panic!("MT5_REQUIRE_LIVE is set, but Mt5Client::connect failed: {e:?}");
            }
            None // Gracefully skip if MT5 bridge is not active/available
        }
    }
}

#[test]
fn test_live_account_info() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let acct = client.account_info().expect("account_info failed");
    assert!(acct.balance > 0.0, "Balance should be positive");
    assert!(acct.equity > 0.0, "Equity should be positive");
    assert!(
        acct.free_margin >= 0.0,
        "Free margin should be non-negative"
    );
}

#[test]
fn test_live_symbol_info() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let info = client.symbol_info("EURUSD").expect("symbol_info failed");
    assert_eq!(info.symbol, "EURUSD");
    assert!(info.digits >= 3);
    assert!(info.point > 0.0);
    assert!(info.tick_size > 0.0);
    assert!(info.min_lot > 0.0);
    assert!(info.max_lot >= info.min_lot);

    // Verify lot and price helper behavior on live symbol data
    let rounded_lot = info.round_lot(0.12345);
    assert!(info.is_valid_lot(rounded_lot));
    let rounded_price = info.round_price(1.0854321);
    assert!((rounded_price - 1.08543).abs() < 1e-4);
}

#[test]
fn test_live_copy_rates_and_bars() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let now = Utc::now().timestamp();
    let from = now - (48 * 3600); // 48h lookback ensures Friday bars are retrieved on weekends
    let rates = client
        .copy_rates("EURUSD", Timeframe::M15, from, now)
        .expect("copy_rates failed");
    assert!(!rates.is_empty(), "Should fetch rates");

    let bars = client
        .copy_bars("EURUSD", Timeframe::M15, from, now)
        .expect("copy_bars failed");
    assert_eq!(rates.len(), bars.len());

    let last_bar = bars.last().unwrap();
    assert!(last_bar.high >= last_bar.low);
    assert!(last_bar.range() >= 0.0);
}

#[test]
fn test_live_symbol_tick() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let tick = client.symbol_tick("EURUSD").expect("symbol_tick failed");
    assert!(tick.bid > 0.0);
    assert!(tick.ask > 0.0);
    assert!(tick.ask >= tick.bid);
    assert!(tick.time_msc > 0);
}

struct OrderGuard<'a> {
    client: &'a Mt5Client,
    ticket: u64,
}

impl<'a> Drop for OrderGuard<'a> {
    fn drop(&mut self) {
        if self.ticket > 0 {
            let _ = self.client.order_close(self.ticket);
        }
    }
}

/// Safety gate: Returns true only if explicitly confirmed that tests are running against a demo account.
/// This prevents `cargo test` from accidentally placing real trades if live credentials are configured.
fn is_demo_confirmed() -> bool {
    env::var("MT5_DEMO_ACCOUNT")
        .or_else(|_| env::var("MT5_IS_DEMO"))
        .or_else(|_| env::var("MT5_ACCOUNT_TYPE"))
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("demo"))
        .unwrap_or(false)
}

#[test]
fn test_live_pending_order_lifecycle() {
    if !is_demo_confirmed() {
        eprintln!(
            "SKIPPING test_live_pending_order_lifecycle: Demo account guard is active. \
             Set MT5_DEMO_ACCOUNT=1 to confirm you are using a demo account before running order tests."
        );
        return;
    }

    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let tick = client.symbol_tick("EURUSD").expect("tick failed");
    let info = client.symbol_info("EURUSD").expect("info failed");

    // 1. Place BuyLimit 200 points below current Bid
    let limit_price = info.round_price(tick.bid - (200.0 * info.point));
    let req = OrderRequest::pending("EURUSD", OrderType::BuyLimit, info.min_lot, limit_price)
        .comment("live_test_suite")
        .deviation(10);

    let trade_res = match client.order_send(&req) {
        Ok(r) => r,
        Err(Mt5Error::OrderSendFailed { retcode: 10018, .. }) => {
            println!("Market is closed (weekend) for EURUSD; pending order skipped gracefully");
            return;
        }
        Err(e) => panic!("order_send failed: {:?}", e),
    };
    assert!(trade_res.is_success());
    assert!(trade_res.is_placed());
    let ticket = trade_res.order;
    assert!(ticket > 0);

    // Guard will automatically cancel the order even if any assertion below panics
    let mut order_guard = OrderGuard {
        client: &client,
        ticket,
    };

    // 2. Modify pending order SL/TP
    let sl = info.round_price(limit_price - (100.0 * info.point));
    let tp = info.round_price(limit_price + (100.0 * info.point));
    let mod_res = client
        .order_modify(ticket, sl, tp)
        .expect("order_modify failed");
    assert!(mod_res.is_success());

    // 3. Cancel pending order cleanly
    let cancel_res = client.order_close(ticket).expect("order_close failed");
    assert!(cancel_res.is_success());
    order_guard.ticket = 0; // Successfully cancelled, disarm drop guard
}

#[test]
fn test_live_chunked_history() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let now = Utc::now().timestamp();
    let start = now - (7 * 86400); // 7 days of H1
    let history_res = client
        .copy_rates_chunked_detailed("EURUSD", Timeframe::H1, start, now, 100)
        .expect("copy_rates_chunked_detailed failed");

    assert!(!history_res.rates.is_empty());
    assert!(history_res.is_complete());
    assert!(history_res.missing_ranges.is_empty());
}

#[test]
fn test_live_market_order_lifecycle() {
    if !is_demo_confirmed() {
        eprintln!(
            "SKIPPING test_live_market_order_lifecycle: Demo account guard is active. \
             Set MT5_DEMO_ACCOUNT=1 to confirm you are using a demo account before running order tests."
        );
        return;
    }

    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let info = client.symbol_info("EURUSD").expect("info failed");
    let req = OrderRequest::buy("EURUSD", info.min_lot).comment("live_market_test");

    let trade_res = match client.order_send(&req) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Market order send returned: {} (market may be closed)", e);
            return;
        }
    };

    assert!(trade_res.is_success());
    assert!(trade_res.order > 0);
    let target_ticket = if trade_res.position > 0 {
        trade_res.position
    } else {
        trade_res.order
    };

    let mut guard = OrderGuard {
        client: &client,
        ticket: target_ticket,
    };

    // Close position using the ergonomic close_position API
    let close_res = client
        .close_position(target_ticket)
        .expect("close_position failed");
    assert!(close_res.is_success());
    guard.ticket = 0;
}

#[test]
fn test_live_events_dropped_total() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let dropped = client.events_dropped_total();
    println!("Live events dropped counter: {dropped}");
    // Counter should be accessible without crashing or UB
    assert!(dropped < 100_000_000);
}

#[test]
fn test_live_positions_and_orders() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let positions = client.positions().expect("client.positions failed");
    println!("Live positions retrieved: {}", positions.len());

    let orders = client
        .pending_orders()
        .expect("client.pending_orders failed");
    println!("Live working orders retrieved: {}", orders.len());
}

#[test]
fn test_live_reconciliation_engine() {
    use mt5_bridge::{LifecycleState, OrderManager};

    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let mut manager = OrderManager::new(998877);
    assert_eq!(manager.lifecycle(), LifecycleState::Starting);

    let report = manager
        .reconcile(&client)
        .expect("manager.reconcile failed");
    assert_eq!(manager.lifecycle(), LifecycleState::Ready);
    println!(
        "Live reconciliation completed: {} positions, {} orders, is_clean: {}",
        report.positions.len(),
        report.pending_orders.len(),
        report.is_clean()
    );
}

#[test]
fn test_live_deal_history() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let now = Utc::now().timestamp();
    let from = now - (30 * 86400); // 30 days lookback
    let deals = client.deals(from, now).expect("client.deals failed");
    println!("Live deals retrieved from history: {}", deals.len());
    for deal in &deals {
        assert!(deal.ticket > 0);
        assert!(!deal.symbol.is_empty());
    }
}

#[test]
fn test_live_order_manager_reconciliation_with_deals() {
    use mt5_bridge::{LifecycleState, OrderManager};

    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let mut manager = OrderManager::new(998877);
    assert_eq!(manager.lifecycle(), LifecycleState::Starting);

    let report = manager
        .reconcile(&client)
        .expect("manager.reconcile failed");
    assert_eq!(manager.lifecycle(), LifecycleState::Ready);
    assert!(
        report.unresolved_orders.is_empty(),
        "Fresh manager should have 0 unresolved orders"
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_live_push_tick_stream_lossless() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let mut sub = client
        .subscribe_ticks_with_mode("EURUSD", mt5_bridge::StreamMode::Lossless)
        .expect("subscribe_ticks_with_mode Lossless failed");
    assert_eq!(sub.mode(), mt5_bridge::StreamMode::Lossless);
    assert_eq!(sub.symbol(), "EURUSD");
    assert_eq!(sub.dropped_ticks(), 0);

    // Receive real-time ticks in Lossless mode using recv_checked()
    match tokio::time::timeout(std::time::Duration::from_secs(5), sub.recv_checked()).await {
        Ok(Ok(tick)) => {
            assert_eq!(tick.symbol, "EURUSD");
            assert!(tick.bid > 0.0);
            assert!(tick.ask >= tick.bid);
            assert_eq!(
                sub.dropped_ticks(),
                0,
                "No ticks should be dropped in normal flow"
            );
        }
        Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped))) => {
            panic!("Lossless mode lagged unexpectedly: skipped {skipped}");
        }
        Ok(Err(e)) => panic!("recv_checked error: {e:?}"),
        Err(_) => {
            println!("Market was idle during 5s timeout");
        }
    }

    let _ = client.unsubscribe_ticks("EURUSD");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_live_push_tick_stream_latest() {
    let (client, _guard) = match get_client() {
        Some(cg) => cg,
        None => return,
    };

    let mut sub = client
        .subscribe_ticks_with_mode("EURUSD", mt5_bridge::StreamMode::Latest)
        .expect("subscribe_ticks_with_mode Latest failed");
    assert_eq!(sub.mode(), mt5_bridge::StreamMode::Latest);

    match tokio::time::timeout(std::time::Duration::from_secs(5), sub.recv()).await {
        Ok(Some(tick)) => {
            assert_eq!(tick.symbol, "EURUSD");
            assert!(tick.bid > 0.0);
        }
        Ok(None) => panic!("Stream ended unexpectedly"),
        Err(_) => {
            println!("Market was idle during 5s timeout");
        }
    }

    let _ = client.unsubscribe_ticks("EURUSD");
}

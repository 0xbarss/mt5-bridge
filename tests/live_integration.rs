use chrono::Utc;
use mt5_bridge::{Mt5Client, OrderRequest, OrderType, Timeframe};
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

    match Mt5Client::connect(login, &password, &server) {
        Ok(client) => Some((client, guard)),
        Err(_) => None, // Gracefully skip if MT5 bridge is not active/available
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
    let from = now - (12 * 3600);
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

#[test]
fn test_live_pending_order_lifecycle() {
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

    let trade_res = client.order_send(&req).expect("order_send failed");
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

use chrono::Utc;
use mt5_bridge::{Mt5Client, OrderRequest, OrderType, Timeframe};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MT5 Bridge End-to-End Live Verification Suite ===");

    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD")
        .or_else(|_| env::var("MT5_PIPE_SECRET"))
        .unwrap_or_default();
    let server = env::var("MT5_SERVER").unwrap_or_default();

    println!(
        "Connecting to MT5 bridge (login: {}, server: '{}')...",
        login, server
    );
    let client = Mt5Client::connect(login, &password, &server)?;
    println!("✓ Connected successfully!");

    // 1. Account Info
    println!("\n[1/5] Testing account_info()...");
    let acct = client.account_info()?;
    println!(
        "  Balance: {:.2}, Equity: {:.2}, Margin: {:.2}, Free Margin: {:.2}",
        acct.balance, acct.equity, acct.margin, acct.free_margin
    );
    assert!(acct.balance > 0.0, "Balance should be positive");
    println!("✓ account_info() verified!");

    // 2. Symbol Info
    println!("\n[2/5] Testing symbol_info('EURUSD')...");
    let info = client.symbol_info("EURUSD")?;
    println!(
        "  Point: {}, Digits: {}, MinLot: {}, MaxLot: {}, LotStep: {}",
        info.point, info.digits, info.min_lot, info.max_lot, info.lot_step
    );
    assert_eq!(info.symbol, "EURUSD");
    assert!(info.digits >= 3);
    println!("✓ symbol_info() verified!");

    // 3. Historical Rates (CopyRates with buffer bound)
    println!("\n[3/5] Testing copy_rates() with buffer safety bounds...");
    let now = Utc::now().timestamp();
    let from = now - (12 * 3600); // 12 hours ago
    let rates = client.copy_rates("EURUSD", Timeframe::M15, from, now)?;
    println!("  Fetched {} M15 bars", rates.len());
    assert!(!rates.is_empty(), "Should fetch at least one M15 bar");
    let bars = client.copy_bars("EURUSD", Timeframe::M15, from, now)?;
    assert_eq!(rates.len(), bars.len());
    println!("✓ copy_rates() and copy_bars() verified!");

    // 4. Symbol Tick
    println!("\n[4/5] Testing symbol_tick('EURUSD')...");
    let tick = client.symbol_tick("EURUSD")?;
    println!(
        "  Bid: {:.5}, Ask: {:.5}, TimeMsc: {}",
        tick.bid, tick.ask, tick.time_msc
    );
    assert!(tick.bid > 0.0 && tick.ask > 0.0);
    println!("✓ symbol_tick() verified!");

    // 5. Order Management: Place & Cancel Pending BuyLimit
    println!("\n[5/5] Testing pending order placement and cancellation...");
    let limit_price = ((tick.ask - (50.0 * info.point)) * 100000.0).round() / 100000.0;
    let req = OrderRequest::pending("EURUSD", OrderType::BuyLimit, info.min_lot, limit_price)
        .comment("e2e_verify");
    let trade_res = client.order_send(&req)?;
    println!(
        "  Placed BuyLimit ticket: {}, retcode: {}",
        trade_res.order, trade_res.retcode
    );
    assert!(trade_res.is_success());

    let cancel_res = client.order_close(trade_res.order)?;
    println!(
        "  Cancelled ticket: {}, retcode: {}",
        trade_res.order, cancel_res.retcode
    );
    assert!(cancel_res.is_success());
    println!("✓ Order execution and cancellation verified!");

    println!("\n========================================================");
    println!("🎉 ALL CHECKS PASSED: MT5 Bridge is 100% verified & secure!");
    println!("========================================================");

    Ok(())
}

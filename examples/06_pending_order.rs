use mt5_bridge::{Mt5Client, OrderRequest, OrderType};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "5056168447".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD").unwrap_or_else(|_| "demo_password".to_string());
    let server = env::var("MT5_SERVER").unwrap_or_else(|_| "MetaQuotes-Demo".to_string());

    let client = Mt5Client::connect(login, &password, &server)?;
    let symbol = "EURUSD";

    // 1. Fetch current price
    let tick = client.symbol_tick(symbol)?;
    println!(
        "Current EURUSD price: Bid={:.5}, Ask={:.5} (time_msc: {})",
        tick.bid, tick.ask, tick.time_msc
    );

    let info = client.symbol_info(symbol)?;
    let point = info.point;
    let volume = info.min_lot;

    // Place BuyLimit 50 points below current Ask
    let limit_price = ((tick.ask - (50.0 * point)) * 100000.0).round() / 100000.0;
    println!(
        "Placing BuyLimit order at {:.5} for {:.2} lots...",
        limit_price, volume
    );

    let req = OrderRequest::pending(symbol, OrderType::BuyLimit, volume, limit_price)
        .comment("test_buylimit");

    let result = client.order_send(&req)?;
    println!("✓ Pending BuyLimit order placed successfully!");
    println!("  Ticket:   {}", result.order);
    println!("  Retcode:  {} ({})", result.retcode, result.description());
    println!("  Price:    {:.5}", result.price);

    // Cancel the pending order by ticket
    println!("Cancelling pending order ticket {}...", result.order);
    let cancel_res = client.order_close(result.order)?;
    println!(
        "✓ Pending order cancelled successfully! (retcode: {})",
        cancel_res.retcode
    );

    Ok(())
}

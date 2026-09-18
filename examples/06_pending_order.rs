use mt5_bridge::{Mt5Client, OrderRequest, OrderType};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD")
        .or_else(|_| env::var("MT5_PIPE_SECRET"))
        .unwrap_or_default();
    let server = env::var("MT5_SERVER").unwrap_or_default();

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

    // Place BuyLimit 200 points below current Bid, rounded to broker tick size
    let limit_price = info.round_price(tick.bid - (200.0 * point));
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
    println!("  Status:   {:?}", result.status());
    println!("  Price:    {:.5}", result.price);

    // Cancel the pending order by ticket
    println!("Cancelling pending order ticket {}...", result.order);
    let cancel_res = client.order_close(result.order)?;
    println!(
        "✓ Pending order cancelled successfully! (retcode: {} - {})",
        cancel_res.retcode,
        cancel_res.description()
    );

    Ok(())
}

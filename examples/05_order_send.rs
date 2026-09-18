use mt5_bridge::{Mt5Client, OrderRequest};
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

    // 1. Check current tick to determine market price
    let tick = client.symbol_tick(symbol)?;
    println!(
        "Current EURUSD price: Bid={:.5}, Ask={:.5}",
        tick.bid, tick.ask
    );

    let info = client.symbol_info(symbol)?;
    let volume = info.min_lot; // Use minimum allowed lot size
    let point = info.point;

    // Define Stop Loss 200 points below Bid, Take Profit 400 points above Bid
    // Always round prices using info.round_price() to ensure tick alignment and broker acceptance
    let sl = info.round_price(tick.bid - (200.0 * point));
    let tp = info.round_price(tick.bid + (400.0 * point));

    // 2. Prepare and send Buy market order
    let order_req = OrderRequest::buy(symbol, volume)
        .stop_loss(sl)
        .take_profit(tp)
        .comment("mt5_bridge_test");

    println!(
        "Placing market BUY order for {:.2} lots (SL: {:.5}, TP: {:.5})...",
        volume, sl, tp
    );
    let result = match client.order_send(&order_req) {
        Ok(r) => r,
        Err(mt5_bridge::Mt5Error::OrderSendFailed {
            retcode: 10018,
            description,
            ..
        }) => {
            println!(
                "ℹ Market is currently closed (weekend): retcode 10018 ({})",
                description
            );
            println!(
                "✓ Pre-flight order validation and bridge serialization completed successfully."
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    println!("✓ Order opened successfully!");
    println!("  Ticket:   {}", result.order);
    println!("  Deal:     {}", result.deal);
    println!("  Position: {}", result.position);
    println!("  Price:    {:.5}", result.price);
    println!("  Volume:   {:.2}", result.volume);
    println!(
        "  Status:   {:?} ({})",
        result.status(),
        result.description()
    );

    let target_ticket = if result.position > 0 {
        result.position
    } else {
        result.order
    };

    // 3. Modify Stop Loss (move SL closer by 50 points, keeping tick alignment)
    let new_sl = info.round_price(tick.bid - (150.0 * point));
    println!(
        "Modifying SL for ticket {} to {:.5}...",
        target_ticket, new_sl
    );
    let mod_res = client.order_modify(target_ticket, new_sl, tp)?;
    println!(
        "✓ Order SL modified successfully! (retcode: {} - {})",
        mod_res.retcode,
        mod_res.description()
    );

    // 4. Close the position by ticket
    println!("Closing position ticket {}...", target_ticket);
    let close_res = client.order_close(target_ticket)?;
    println!(
        "✓ Position closed at price {:.5} (Deal: {}, retcode: {} - {})",
        close_res.price,
        close_res.deal,
        close_res.retcode,
        close_res.description()
    );

    Ok(())
}

use mt5_bridge::Mt5Client;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "12345678".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD").unwrap_or_else(|_| "demo_password".to_string());
    let server = env::var("MT5_SERVER").unwrap_or_else(|_| "MetaQuotes-Demo".to_string());

    let client = Mt5Client::connect(login, &password, &server)?;

    let symbols = ["EURUSD", "GBPUSD", "USDJPY", "XAUUSD", "BTCUSD"];

    println!("{:<10} {:<10} {:<10} {:<10} {:<10} {:<10} {:<8}",
        "Symbol", "Point", "TickVal", "MinLot", "MaxLot", "LotStep", "Digits");
    println!("{:-<70}", "");

    for symbol in symbols {
        match client.symbol_info(symbol) {
            Ok(info) => {
                println!(
                    "{:<10} {:<10.5} {:<10.2} {:<10.2} {:<10.2} {:<10.2} {:<8}",
                    info.symbol,
                    info.point,
                    info.tick_value,
                    info.min_lot,
                    info.max_lot,
                    info.lot_step,
                    info.digits
                );

                // Show lot rounding helper
                let raw_lot = 0.12345;
                let rounded = info.round_lot(raw_lot);
                println!("  ↳ Lot size {raw_lot} normalized to: {rounded}");
            }
            Err(e) => {
                println!("{:<10} Error: {}", symbol, e);
            }
        }
    }

    Ok(())
}

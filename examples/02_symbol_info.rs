use mt5_bridge::Mt5Client;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "0".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD")
        .or_else(|_| env::var("MT5_PIPE_SECRET"))
        .unwrap_or_default();
    let server = env::var("MT5_SERVER").unwrap_or_default();

    let mut client = Mt5Client::connect(login, &password, &server)?;
    // Configure in-memory cache TTL for symbol specifications
    client.set_symbol_cache_ttl(std::time::Duration::from_secs(120));

    let symbols = ["EURUSD", "GBPUSD", "USDJPY", "XAUUSD", "BTCUSD"];

    println!(
        "{:<10} {:<10} {:<10} {:<10} {:<10} {:<10} {:<8}",
        "Symbol", "Point", "TickVal", "MinLot", "MaxLot", "LotStep", "Digits"
    );
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

                // Show lot normalization & risk calculation helpers
                let raw_lot = 0.12345;
                let rounded = info.round_lot(raw_lot);
                let is_valid = info.is_valid_lot(rounded);
                let p_val = info.point_value(rounded);
                println!(
                    "  ↳ Lot {raw_lot} -> rounded: {rounded} (valid: {is_valid}) | 1-point move value: ${p_val:.5}"
                );
            }
            Err(e) => {
                println!("{:<10} Error: {}", symbol, e);
            }
        }
    }

    Ok(())
}

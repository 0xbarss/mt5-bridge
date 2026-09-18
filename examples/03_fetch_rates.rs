use chrono::{TimeZone, Utc};
use mt5_bridge::{Mt5Client, Timeframe};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "12345678".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD").unwrap_or_else(|_| "demo_password".to_string());
    let server = env::var("MT5_SERVER").unwrap_or_else(|_| "MetaQuotes-Demo".to_string());

    let client = Mt5Client::connect(login, &password, &server)?;

    let symbol = "EURUSD";
    let timeframe = Timeframe::M15;

    // Fetch bars for the last 24 hours
    let now = Utc::now().timestamp();
    let from = now - (24 * 3600);

    println!("Fetching {} {} bars from {} to {}...", symbol, timeframe, from, now);
    let bars = client.copy_bars(symbol, timeframe, from, now)?;

    println!("Fetched {} bars total. Last 10 bars:", bars.len());
    println!("{:<20} {:<10} {:<10} {:<10} {:<10} {:<10}",
        "Time (UTC)", "Open", "High", "Low", "Close", "Volume");
    println!("{:-<72}", "");

    for bar in bars.iter().rev().take(10).rev() {
        let dt = Utc.timestamp_opt(bar.time, 0).single().unwrap_or_default();
        println!(
            "{:<20} {:<10.5} {:<10.5} {:<10.5} {:<10.5} {:<10.0}",
            dt.format("%Y-%m-%d %H:%M"),
            bar.open,
            bar.high,
            bar.low,
            bar.close,
            bar.volume
        );
    }

    Ok(())
}

use chrono::{TimeZone, Utc};
use mt5_bridge::{Mt5Client, Timeframe};
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
    let timeframe = Timeframe::M15;

    // Fetch bars for the last 24 hours
    let now = Utc::now().timestamp();
    let from = now - (24 * 3600);

    println!(
        "1. Fetching raw {} {} rates (with spread) from {} to {}...",
        symbol, timeframe, from, now
    );
    let rates = client.copy_rates(symbol, timeframe, from, now)?;
    println!("Fetched {} raw rates.", rates.len());
    if let Some(r) = rates.last() {
        println!(
            "   Latest raw rate: Open={:.5}, Close={:.5}, Spread={} points, Vol={}",
            r.open, r.close, r.spread, r.volume
        );
    }

    println!(
        "\n2. Fetching clean {} {} bars with technical analysis metrics...",
        symbol, timeframe
    );
    let bars = client.copy_bars(symbol, timeframe, from, now)?;
    println!("Fetched {} bars total. Sample of last 5 bars:", bars.len());
    println!(
        "{:<18} {:<9} {:<9} {:<9} {:<9} {:<9} {:<9} {:<8}",
        "Time (UTC)", "Open", "High", "Low", "Close", "Mid", "Range", "Trend"
    );
    println!("{:-<85}", "");

    for bar in bars.iter().rev().take(5).rev() {
        let dt = Utc.timestamp_opt(bar.time, 0).single().unwrap_or_default();
        let trend = if bar.is_bullish() {
            "BULL"
        } else if bar.is_bearish() {
            "BEAR"
        } else {
            "FLAT"
        };
        println!(
            "{:<18} {:<9.5} {:<9.5} {:<9.5} {:<9.5} {:<9.5} {:<9.5} {:<8}",
            dt.format("%Y-%m-%d %H:%M"),
            bar.open,
            bar.high,
            bar.low,
            bar.close,
            bar.mid(),
            bar.range(),
            trend
        );
    }

    Ok(())
}

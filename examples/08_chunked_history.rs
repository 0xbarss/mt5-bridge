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

    println!(
        "Connecting to MT5 bridge (login: {}, server: '{}')...",
        login, server
    );
    let client = Mt5Client::connect(login, &password, &server)?;

    let symbol = "EURUSD";
    let timeframe = Timeframe::H1;

    // Request 30 days of H1 bars in chunks of 200 bars
    let now = Utc::now().timestamp();
    let start = now - (30 * 86400); // 30 days ago
    let chunk_bars = 200;

    println!(
        "Downloading deep history for {} {} in chunks of {} bars...",
        symbol, timeframe, chunk_bars
    );
    println!("Time range: {} to {}", start, now);

    let history_res =
        client.copy_rates_chunked_detailed(symbol, timeframe, start, now, chunk_bars)?;

    println!(
        "✓ Successfully downloaded {} historical bars total! (Complete: {}, missing chunks: {})",
        history_res.rates.len(),
        history_res.is_complete(),
        history_res.missing_ranges.len()
    );

    let rates = history_res.rates;

    if let Some(first) = rates.first() {
        let dt_first = Utc
            .timestamp_opt(first.time, 0)
            .single()
            .unwrap_or_default();
        println!(
            "  Earliest Bar: {} | Open: {:.5} | Close: {:.5}",
            dt_first.format("%Y-%m-%d %H:%M"),
            first.open,
            first.close
        );
    }

    if let Some(last) = rates.last() {
        let dt_last = Utc.timestamp_opt(last.time, 0).single().unwrap_or_default();
        println!(
            "  Latest Bar:   {} | Open: {:.5} | Close: {:.5}",
            dt_last.format("%Y-%m-%d %H:%M"),
            last.open,
            last.close
        );
    }

    println!("\nSample of last 5 bars:");
    println!(
        "{:<18} {:<10} {:<10} {:<10} {:<10} {:<8}",
        "Timestamp (UTC)", "Open", "High", "Low", "Close", "Volume"
    );
    println!("{:-<68}", "");
    for r in rates.iter().rev().take(5).rev() {
        let dt = Utc.timestamp_opt(r.time, 0).single().unwrap_or_default();
        println!(
            "{:<18} {:<10.5} {:<10.5} {:<10.5} {:<10.5} {:<8}",
            dt.format("%Y-%m-%d %H:%M"),
            r.open,
            r.high,
            r.low,
            r.close,
            r.volume
        );
    }

    Ok(())
}

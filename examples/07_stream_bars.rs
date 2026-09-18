use chrono::{TimeZone, Utc};
use mt5_bridge::{stream_bars, Mt5Client, Timeframe};
use std::env;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let client = Arc::new(Mt5Client::connect(login, &password, &server)?);

    let symbol = "EURUSD";
    let timeframe = Timeframe::M1;

    println!(
        "Starting real-time closed-bar stream for {} {}...",
        symbol, timeframe
    );
    println!("(Listening for completed candles; poll interval: 1s)");

    let mut rx = stream_bars(client, symbol, timeframe, Duration::from_secs(1));

    let mut count = 0;
    while let Some(bar) = rx.recv().await {
        let dt = Utc.timestamp_opt(bar.time, 0).single().unwrap_or_default();
        let trend = if bar.is_bullish() {
            "BULLISH"
        } else if bar.is_bearish() {
            "BEARISH"
        } else {
            "DOJI"
        };

        println!(
            "[{}] Closed Bar -> Open: {:.5} | High: {:.5} | Low: {:.5} | Close: {:.5} | Range: {:.5} | Vol: {:.0} [{}]",
            dt.format("%Y-%m-%d %H:%M:%S"),
            bar.open,
            bar.high,
            bar.low,
            bar.close,
            bar.range(),
            bar.volume,
            trend
        );

        count += 1;
        if count >= 1 {
            println!("Received {} closed bar. Exiting stream.", count);
            break;
        }
    }

    Ok(())
}

use chrono::{TimeZone, Utc};
use mt5_bridge::{Mt5Client, Timeframe};
use std::env;
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
        "Connecting to MT5 bridge (login: {}, server: '{}', Protocol v7)...",
        login, server
    );
    let client = Mt5Client::connect(login, &password, &server)?;

    let symbol = "EURUSD";
    let timeframe = Timeframe::M1;

    println!(
        "Subscribing to push bar events for {} {} (Protocol v7 async push)...",
        symbol, timeframe
    );

    let mut rx = client.subscribe_bars(symbol, timeframe)?;

    println!("Listening for bar push notifications (timeout: 5s)...");

    match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        Ok(Ok(bar)) => {
            let dt = Utc.timestamp_opt(bar.time, 0).single().unwrap_or_default();
            let trend = if bar.is_bullish() {
                "BULLISH"
            } else if bar.is_bearish() {
                "BEARISH"
            } else {
                "DOJI"
            };

            println!(
                "[{}] Push Bar -> Open: {:.5} | High: {:.5} | Low: {:.5} | Close: {:.5} | Volume: {:.0} | Range: {:.5} [{}]",
                dt.format("%Y-%m-%d %H:%M:%S"),
                bar.open,
                bar.high,
                bar.low,
                bar.close,
                bar.volume,
                bar.range(),
                trend
            );
        }
        Ok(Err(e)) => {
            eprintln!("Broadcast receive error: {e}");
        }
        Err(_) => {
            println!(
                "No bar push events received within 5s timeout (market may be idle or closed)."
            );
        }
    }

    client.unsubscribe_bars(symbol, timeframe)?;
    println!("Unsubscribed from bar events. Done.");

    Ok(())
}

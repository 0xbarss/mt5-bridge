use chrono::{TimeZone, Utc};
use mt5_bridge::{stream_ticks, Mt5Client};
use std::env;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let login: i64 = env::var("MT5_LOGIN")
        .unwrap_or_else(|_| "12345678".to_string())
        .parse()?;
    let password = env::var("MT5_PASSWORD").unwrap_or_else(|_| "demo_password".to_string());
    let server = env::var("MT5_SERVER").unwrap_or_else(|_| "MetaQuotes-Demo".to_string());

    let client = Arc::new(Mt5Client::connect(login, &password, &server)?);
    let symbol = "EURUSD";

    println!("Starting real-time tick stream for {}...", symbol);
    let mut rx = stream_ticks(client, symbol, Duration::from_millis(10));

    let mut count = 0;
    while let Some(tick) = rx.recv().await {
        let dt = Utc.timestamp_opt(tick.time, 0).single().unwrap_or_default();
        println!(
            "[{}] Tick: Bid={:.5} Ask={:.5} Spread={:.5} Last={:.5} Vol={}",
            dt.format("%H:%M:%S"),
            tick.bid,
            tick.ask,
            tick.spread(),
            tick.last,
            tick.volume
        );

        count += 1;
        if count >= 20 {
            println!("Received 20 ticks. Dropping receiver to stop background task.");
            break;
        }
    }

    Ok(())
}

use chrono::{TimeZone, Utc};
use mt5_bridge::{stream_ticks, Mt5Client};
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

    let client = Arc::new(Mt5Client::connect(login, &password, &server)?);
    let symbol = "EURUSD";

    println!("Starting real-time tick stream for {}...", symbol);
    let mut rx = stream_ticks(client, symbol, Duration::from_millis(10));

    let mut count = 0;
    while count < 5 {
        match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Some(tick)) => {
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
                if count >= 5 {
                    println!("Received 5 ticks. Dropping receiver to stop background task.");
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => {
                println!(
                    "No new ticks received within 2s (market may be closed/idle). Stopping stream."
                );
                break;
            }
        }
    }

    Ok(())
}

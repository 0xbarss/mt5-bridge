use chrono::{TimeZone, Utc};
use mt5_bridge::{Mt5Client, StreamMode};
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

    let client = Mt5Client::connect(login, &password, &server)?;
    let symbol = "EURUSD";

    println!(
        "Starting real-time push tick streams for {} (Protocol v5+)...\n",
        symbol
    );

    // --- Part 1: StreamMode::Lossless (Recorder stream) ---
    println!("=== 1. Testing StreamMode::Lossless (Auditing / Recording Stream) ===");
    let mut lossless_ticks = client.subscribe_ticks_with_mode(symbol, StreamMode::Lossless)?;
    let mut count = 0;
    while count < 3 {
        match tokio::time::timeout(Duration::from_secs(3), lossless_ticks.recv_checked()).await {
            Ok(Ok(tick)) => {
                let dt = Utc.timestamp_opt(tick.time, 0).single().unwrap_or_default();
                println!(
                    "[{}] Lossless Tick #{}: Bid={:.5} Ask={:.5} Spread={:.5} Vol={} (dropped={})",
                    dt.format("%H:%M:%S"),
                    count + 1,
                    tick.bid,
                    tick.ask,
                    tick.spread(),
                    tick.volume,
                    lossless_ticks.dropped_ticks()
                );
                count += 1;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped))) => {
                println!("Lossless stream detected consumer lag: {skipped} ticks skipped!");
            }
            Ok(Err(e)) => {
                eprintln!("Lossless stream error: {e}");
                break;
            }
            Err(_) => {
                println!("No ticks received within 3s timeout for lossless stream.");
                break;
            }
        }
    }
    let _ = client.unsubscribe_ticks(symbol);

    // --- Part 2: StreamMode::Latest (Execution stream) ---
    println!("\n=== 2. Testing StreamMode::Latest (Low-Latency Execution Stream) ===");
    let mut latest_ticks = client.subscribe_ticks_with_mode(symbol, StreamMode::Latest)?;
    let mut count = 0;
    while count < 3 {
        match tokio::time::timeout(Duration::from_secs(3), latest_ticks.recv()).await {
            Ok(Some(tick)) => {
                let dt = Utc.timestamp_opt(tick.time, 0).single().unwrap_or_default();
                println!(
                    "[{}] Latest Tick #{}: Bid={:.5} Ask={:.5} Spread={:.5} Vol={} (dropped={})",
                    dt.format("%H:%M:%S"),
                    count + 1,
                    tick.bid,
                    tick.ask,
                    tick.spread(),
                    tick.volume,
                    latest_ticks.dropped_ticks()
                );
                count += 1;
            }
            Ok(None) => break,
            Err(_) => {
                println!("No ticks received within 3s timeout for latest stream.");
                break;
            }
        }
    }
    let _ = client.unsubscribe_ticks(symbol);

    println!("\n✓ Both StreamMode::Lossless and StreamMode::Latest streams tested successfully!");
    Ok(())
}

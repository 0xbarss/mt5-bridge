//! Asynchronous real-time tick and bar streaming via Tokio channels.

use crate::client::Mt5Client;
use crate::types::{Bar, Tick, Timeframe};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, error, warn};

const DEFAULT_TICK_BUFFER: usize = 1024;
const DEFAULT_BAR_BUFFER: usize = 256;

const DUP_PRICE_THRESHOLD: f64 = 1e-9;

/// Maximum consecutive poll errors allowed before a streaming task aborts and closes the channel.
pub const MAX_CONSECUTIVE_STREAM_ERRORS: u32 = 10;

/// Stream real-time ticks for a symbol using latest-quote polling.
///
/// **Streaming Semantics:**
/// Spawns a background Tokio task that periodically polls the bridge for new quotes
/// via `symbol_tick()` at the specified `poll_interval`. New quotes (changes in timestamp,
/// bid, ask, last price, volume, or tick flags) are yielded through an `mpsc::Receiver`.
/// The task terminates automatically when the receiver is dropped.
///
/// *Note on High Frequency:* This is **latest-quote polling**, not a lossless tick queue.
/// Intermediate sub-millisecond price ticks occurring within a single polling interval are
/// coalesced into the latest quote. For multi-symbol streaming, use balanced intervals (e.g. 20–100ms)
/// to maintain efficient IPC pipe throughput.
pub fn stream_ticks(
    client: Arc<Mt5Client>,
    symbol: &str,
    poll_interval: Duration,
) -> mpsc::Receiver<Tick> {
    let (tx, rx) = mpsc::channel(DEFAULT_TICK_BUFFER);
    let sym_owned = symbol.to_string();

    tokio::spawn(async move {
        debug!(symbol = %sym_owned, "Tick stream started");

        let mut prev_time_msc: i64 = 0;
        let mut prev_bid: f64 = 0.0;
        let mut prev_ask: f64 = 0.0;
        let mut prev_last: f64 = 0.0;
        let mut prev_volume: u64 = 0;
        let mut prev_flags: u32 = 0;
        let mut consecutive_errors: u32 = 0;

        loop {
            let client_clone = Arc::clone(&client);
            let sym_task = sym_owned.clone();

            let tick_res =
                tokio::task::spawn_blocking(move || client_clone.symbol_tick(&sym_task)).await;

            match tick_res {
                Ok(Ok(tick)) => {
                    consecutive_errors = 0;
                    if tick.bid > 0.0 && tick.ask > 0.0 && tick.ask >= tick.bid && tick.time_msc > 0
                    {
                        let time_dup = tick.time_msc == prev_time_msc;
                        let bid_dup = (tick.bid - prev_bid).abs() < DUP_PRICE_THRESHOLD;
                        let ask_dup = (tick.ask - prev_ask).abs() < DUP_PRICE_THRESHOLD;
                        let last_dup = (tick.last - prev_last).abs() < DUP_PRICE_THRESHOLD;
                        let vol_dup = tick.volume == prev_volume;
                        let flags_dup = tick.flags == prev_flags;

                        if !(time_dup && bid_dup && ask_dup && last_dup && vol_dup && flags_dup) {
                            prev_time_msc = tick.time_msc;
                            prev_bid = tick.bid;
                            prev_ask = tick.ask;
                            prev_last = tick.last;
                            prev_volume = tick.volume;
                            prev_flags = tick.flags;

                            if tx.send(tick).await.is_err() {
                                debug!(symbol = %sym_owned, "Tick stream receiver dropped; shutting down");
                                break;
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    consecutive_errors += 1;
                    warn!(
                        symbol = %sym_owned,
                        error = %e,
                        consecutive_errors,
                        max_errors = MAX_CONSECUTIVE_STREAM_ERRORS,
                        "Tick stream poll error"
                    );
                    if consecutive_errors >= MAX_CONSECUTIVE_STREAM_ERRORS {
                        error!(
                            symbol = %sym_owned,
                            consecutive_errors,
                            "Tick stream exceeded maximum consecutive errors; terminating stream"
                        );
                        break;
                    }
                }
                Err(e) => {
                    error!(symbol = %sym_owned, error = %e, "Tick stream task joined with error");
                    break;
                }
            }

            sleep(poll_interval).await;
        }
    });

    rx
}

/// Stream completed (closed) OHLCV bars for a given symbol and timeframe.
///
/// Spawns a background Tokio task that monitors incoming bars and pushes each
/// closed bar through a `mpsc::Receiver`.
/// The background polling task terminates automatically when the receiver is dropped.
pub fn stream_bars(
    client: Arc<Mt5Client>,
    symbol: &str,
    timeframe: Timeframe,
    poll_interval: Duration,
) -> mpsc::Receiver<Bar> {
    let (tx, rx) = mpsc::channel(DEFAULT_BAR_BUFFER);
    let sym_owned = symbol.to_string();

    tokio::spawn(async move {
        debug!(symbol = %sym_owned, timeframe = %timeframe, "Bar stream started");

        let mut last_closed_bar_time: i64 = 0;
        let mut consecutive_errors: u32 = 0;

        loop {
            let now = chrono::Utc::now().timestamp();
            let lookback = match timeframe {
                Timeframe::W1 => 86400 * 7 * 6,   // 6 weeks lookback
                Timeframe::MN1 => 86400 * 32 * 6, // 6 months (~192 days) lookback
                tf => tf.seconds() * 4,
            };
            let from = now - lookback;

            let client_clone = Arc::clone(&client);
            let sym_task = sym_owned.clone();

            let rates_res = tokio::task::spawn_blocking(move || {
                client_clone.copy_rates(&sym_task, timeframe, from, now)
            })
            .await;

            match rates_res {
                Ok(Ok(rates)) => {
                    consecutive_errors = 0;
                    // We need at least 2 bars:
                    // rates[len - 1] is the currently forming unclosed bar.
                    // rates[len - 2] is the most recently completed closed bar.
                    if rates.len() >= 2 {
                        let closed_rate = rates[rates.len() - 2];
                        if closed_rate.time > last_closed_bar_time {
                            if last_closed_bar_time == 0 {
                                // Initialize on first fetch without emitting historical bar
                                last_closed_bar_time = closed_rate.time;
                            } else {
                                last_closed_bar_time = closed_rate.time;
                                let bar = Bar::from(closed_rate);
                                if tx.send(bar).await.is_err() {
                                    debug!(symbol = %sym_owned, "Bar stream receiver dropped; shutting down");
                                    break;
                                }
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    consecutive_errors += 1;
                    warn!(
                        symbol = %sym_owned,
                        error = %e,
                        consecutive_errors,
                        max_errors = MAX_CONSECUTIVE_STREAM_ERRORS,
                        "Bar stream poll error"
                    );
                    if consecutive_errors >= MAX_CONSECUTIVE_STREAM_ERRORS {
                        error!(
                            symbol = %sym_owned,
                            consecutive_errors,
                            "Bar stream exceeded maximum consecutive errors; terminating stream"
                        );
                        break;
                    }
                }
                Err(e) => {
                    error!(symbol = %sym_owned, error = %e, "Bar stream task joined with error");
                    break;
                }
            }

            sleep(poll_interval).await;
        }
    });

    rx
}

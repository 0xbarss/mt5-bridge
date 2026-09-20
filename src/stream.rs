//! Asynchronous real-time tick and bar streaming via Tokio channels.
//!
//! # Delivery semantics — read before relying on a stream
//!
//! These streams are **best-effort, latest-quote polling feeds for monitoring, UI and signal
//! generation. They are not a lossless tick recorder** and must not be used to reconstruct a
//! complete tick history. (The bridge exposes only the latest quote via
//! [`Mt5Client::symbol_tick`](crate::Mt5Client::symbol_tick) and bar history via
//! [`Mt5Client::copy_rates`](crate::Mt5Client::copy_rates); it has no tick-history API, so use
//! MetaTrader's own tick export if you need every tick.)
//! Data can be lost at three independent layers:
//!
//! 1. **Polling gaps (both policies, always).** The task polls the *latest* quote every
//!    [`StreamConfig::poll_interval`] (default 50 ms). Every tick the terminal received
//!    *between* two polls is never seen at all. Lowering the interval narrows the gap but never
//!    closes it.
//! 2. **Duplicate suppression.** A poll that returns a quote identical to the previous one
//!    (same time, bid, ask, last, volume and flags) is not emitted.
//! 3. **Backpressure** (only when the consumer is slower than the feed; see below).
//!
//! ## [`BackpressurePolicy::DropLatest`] (default)
//!
//! When the channel buffer is full the **newly polled tick is discarded** (the *newest* one),
//! while everything already buffered is kept and delivered in order. Despite the name this is
//! *not* "keep only the latest value": a lagging consumer keeps receiving the **oldest,
//! stale** ticks first, then observes a gap, then resumes with fresh ticks once it catches up.
//! Consequences:
//!
//! * A slow consumer can act on prices that are up to `buffer_size × poll_interval` old
//!   (about 51 s with the defaults of 1024 × 50 ms) even though newer quotes were fetched.
//! * Dropped ticks are counted in a `warn!` log (first, then every 100th) but are **not**
//!   reported to the consumer; a gap in `time_msc` is the only in-band signal.
//! * A dropped tick still updates the duplicate-suppression state, so it is not re-offered.
//!
//! Choose it when **loss is preferable to blocking the poller**. If you need the freshest
//! quote, drain the receiver (`while let Ok(t) = rx.try_recv() { latest = t }`) before acting.
//!
//! ## [`BackpressurePolicy::Block`]
//!
//! The polling task awaits free buffer space, so no *already polled* tick is discarded and
//! delivery stays in order — but polling pauses while blocked, so ticks that arrive during the
//! stall are **skipped** (layer 1 grows to the length of the stall). It trades data currency
//! for completeness of what was polled; it does not make the feed lossless.
//!
//! ## Bar streams
//!
//! [`stream_bars`] always blocks on a full channel (no drop policy). It emits each **closed**
//! bar once, does not emit the bar that was already closed when the stream started, and only
//! ever looks at the most recently closed bar per poll — if two bars close within one poll
//! interval (or while a send is blocked) the earlier one is skipped. Keep `poll_interval`
//! well below the timeframe and consume promptly.
//!
//! ## Termination
//!
//! A stream ends (the receiver returns `None`) when the receiver is dropped, or after
//! [`MAX_CONSECUTIVE_STREAM_ERRORS`] consecutive poll failures. Treat `None` as "feed lost",
//! not "no more data", and re-establish it.

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

/// Backpressure policy for stream buffer overflow.
///
/// See the [module documentation](self#delivery-semantics--read-before-relying-on-a-stream)
/// for the full loss model. Neither policy makes the feed lossless: the stream polls the
/// latest quote, so ticks between polls are never observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressurePolicy {
    /// When the buffer is full, **discard the newly polled (newest) tick** and keep the buffered
    /// backlog. The poller never blocks, but a lagging consumer sees stale ticks first and then a
    /// gap — this is *not* "latest value wins". Dropped ticks are only logged, not signalled.
    DropLatest,
    /// Await free buffer space. No polled tick is discarded and order is preserved, but polling
    /// pauses meanwhile, so quotes that arrive during the stall are skipped rather than queued.
    Block,
}

/// Configuration options for market data streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    pub buffer_size: usize,
    pub poll_interval: Duration,
    pub backpressure: BackpressurePolicy,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            buffer_size: DEFAULT_TICK_BUFFER,
            poll_interval: Duration::from_millis(50),
            backpressure: BackpressurePolicy::DropLatest,
        }
    }
}

/// Outcome of offering one tick to the channel under [`BackpressurePolicy::DropLatest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offer {
    Delivered,
    /// Buffer full: the offered (newest) tick was discarded; the backlog is untouched.
    DroppedNewest,
    ReceiverClosed,
}

fn offer_drop_newest(tx: &mpsc::Sender<Tick>, tick: Tick) -> Offer {
    match tx.try_send(tick) {
        Ok(()) => Offer::Delivered,
        Err(mpsc::error::TrySendError::Full(_)) => Offer::DroppedNewest,
        Err(mpsc::error::TrySendError::Closed(_)) => Offer::ReceiverClosed,
    }
}

/// Stream real-time ticks for a symbol using explicit configuration and backpressure policy.
pub fn stream_ticks_with_config(
    client: Arc<Mt5Client>,
    symbol: &str,
    config: StreamConfig,
) -> mpsc::Receiver<Tick> {
    let (tx, rx) = mpsc::channel(config.buffer_size.max(16));
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
        let mut dropped_ticks: u64 = 0;

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

                            match config.backpressure {
                                BackpressurePolicy::DropLatest => match offer_drop_newest(&tx, tick) {
                                    Offer::Delivered => {}
                                    Offer::DroppedNewest => {
                                        dropped_ticks += 1;
                                        if dropped_ticks % 100 == 1 {
                                            warn!(
                                                symbol = %sym_owned,
                                                dropped_ticks,
                                                "Tick stream buffer full: discarded the newest tick (buffered backlog kept); consumer is lagging"
                                            );
                                        }
                                    }
                                    Offer::ReceiverClosed => {
                                        debug!(symbol = %sym_owned, "Tick stream receiver dropped; shutting down");
                                        break;
                                    }
                                },
                                BackpressurePolicy::Block => {
                                    if tx.send(tick).await.is_err() {
                                        debug!(symbol = %sym_owned, "Tick stream receiver dropped; shutting down");
                                        break;
                                    }
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

            sleep(config.poll_interval).await;
        }
    });

    rx
}

/// Stream real-time ticks for a symbol using latest-quote polling and [`BackpressurePolicy::DropLatest`]
/// (newest tick discarded when the buffer is full). Best-effort, not lossless — see the
/// [module docs](self#delivery-semantics--read-before-relying-on-a-stream).
pub fn stream_ticks(
    client: Arc<Mt5Client>,
    symbol: &str,
    poll_interval: Duration,
) -> mpsc::Receiver<Tick> {
    let config = StreamConfig {
        buffer_size: DEFAULT_TICK_BUFFER,
        poll_interval,
        backpressure: BackpressurePolicy::DropLatest,
    };
    stream_ticks_with_config(client, symbol, config)
}

/// Stream completed (closed) OHLCV bars for a given symbol and timeframe.
///
/// Spawns a background Tokio task that monitors incoming bars and pushes each
/// closed bar through a `mpsc::Receiver`.
/// The background polling task terminates automatically when the receiver is dropped.
///
/// A bar is skipped if more than one bar closes within a single poll interval; the bar that was
/// already closed at start-up is not emitted. See the
/// [module docs](self#delivery-semantics--read-before-relying-on-a-stream).
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(n: i64) -> Tick {
        Tick {
            symbol: "EURUSD".into(),
            time_msc: n,
            time: n / 1000,
            bid: 1.0,
            ask: 1.0001,
            last: 0.0,
            volume: 0,
            flags: 0,
        }
    }

    /// Pins the documented `DropLatest` semantics: a full buffer discards the NEWEST tick and the
    /// consumer receives the OLDEST backlog first (not "latest value wins").
    #[test]
    fn drop_latest_discards_the_newest_tick_and_keeps_the_stale_backlog() {
        let (tx, mut rx) = mpsc::channel::<Tick>(4);
        for n in 1..=4 {
            assert_eq!(offer_drop_newest(&tx, tick(n)), Offer::Delivered);
        }
        // buffer full: 5 and 6 are the newest quotes and are the ones that get discarded
        assert_eq!(offer_drop_newest(&tx, tick(5)), Offer::DroppedNewest);
        assert_eq!(offer_drop_newest(&tx, tick(6)), Offer::DroppedNewest);

        let received: Vec<i64> = std::iter::from_fn(|| rx.try_recv().ok()).map(|t| t.time_msc).collect();
        assert_eq!(received, vec![1, 2, 3, 4], "consumer sees the stale prefix, never 5 or 6");

        // once drained the feed resumes, leaving a gap (5, 6) as the only in-band signal
        assert_eq!(offer_drop_newest(&tx, tick(7)), Offer::Delivered);
        assert_eq!(rx.try_recv().unwrap().time_msc, 7);
    }

    #[test]
    fn drop_latest_reports_a_closed_receiver() {
        let (tx, rx) = mpsc::channel::<Tick>(4);
        drop(rx);
        assert_eq!(offer_drop_newest(&tx, tick(1)), Offer::ReceiverClosed);
    }

    #[test]
    fn default_config_is_drop_latest_with_the_documented_buffer_and_interval() {
        let c = StreamConfig::default();
        assert_eq!(c.backpressure, BackpressurePolicy::DropLatest);
        assert_eq!(c.buffer_size, 1024);
        assert_eq!(c.poll_interval, Duration::from_millis(50));
    }
}

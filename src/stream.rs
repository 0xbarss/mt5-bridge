//! Asynchronous real-time market data streaming and event dispatch.
//!
//! # Streaming Architecture (Protocol v5+ Push Model)
//!
//! On protocol v5+ bridges, market data ticks, trade transaction updates, and depth-of-market (DOM)
//! book events are pushed natively from MetaTrader 5 over a full-duplex named pipe into the
//! client's [`EventBus`].
//!
//! ## Push Streams ([`TickSubscription`])
//!
//! Subscribers obtain active push feeds via [`Mt5Client::subscribe_ticks`](crate::Mt5Client::subscribe_ticks)
//! or [`Mt5Client::subscribe_ticks_with_mode`](crate::Mt5Client::subscribe_ticks_with_mode), choosing between two
//! distinct delivery semantics:
//!
//! 1. **[`StreamMode::Latest`] (default for trading)**:
//!    Optimized for low-latency strategy execution. If a consumer falls behind the broadcast buffer,
//!    intermediate quotes are skipped so the consumer always receives the freshest available price.
//!    Total skipped quotes are tracked via [`TickSubscription::dropped_ticks`].
//! 2. **[`StreamMode::Lossless`] (auditing & recording)**:
//!    Designed for tick capture and auditing. Consumers should use [`TickSubscription::recv_checked`]
//!    which returns [`broadcast::error::RecvError::Lagged`] whenever buffer overrun occurs, allowing
//!    the recorder to flag quote gaps explicitly. Calling [`TickSubscription::recv`] on a lossless
//!    stream will log a warning and resume with the next available quote.
//!
//! ## Legacy Polling Streams
//!
//! For environments or fallback configurations where native push is not enabled, the bridge provides
//! polling-based feeds ([`stream_ticks`], [`stream_bars`]) backed by periodic queries.
//!
//! * [`stream_bars`] emits newly closed bars per polling interval.
//! * Polling ticks use [`StreamConfig::poll_interval`] (default 50 ms) with configurable
//!   [`BackpressurePolicy`].
//!
//! ## Termination
//!
//! Push streams terminate (`recv()` returns `None`) when the client disconnects or is dropped. Treat
//! `None` as feed termination and reconnect.

use crate::client::Mt5Client;
use crate::types::{Bar, BookEvent, StreamMode, Tick, Timeframe, TradeEvent};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, warn};

/// Central market data and event dispatcher bus (protocol v5+ push model).
///
/// Dispatches incoming ticks to per-symbol Tokio broadcast channels,
/// and trade/book events to dedicated event broadcast channels.
pub struct EventBus {
    ticks: Mutex<HashMap<String, broadcast::Sender<Tick>>>,
    trades: broadcast::Sender<TradeEvent>,
    books: Mutex<HashMap<String, broadcast::Sender<BookEvent>>>,
    buffer_capacity: usize,
}

impl EventBus {
    pub fn new(buffer_capacity: usize) -> Self {
        let (trades, _) = broadcast::channel(buffer_capacity.max(64));
        Self {
            ticks: Mutex::new(HashMap::new()),
            trades,
            books: Mutex::new(HashMap::new()),
            buffer_capacity: buffer_capacity.max(64),
        }
    }

    /// Subscribe to real-time pushed ticks for a symbol.
    pub fn subscribe_ticks(&self, symbol: &str) -> broadcast::Receiver<Tick> {
        let mut map = self.ticks.lock().unwrap();
        let sender = map.entry(symbol.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(self.buffer_capacity);
            tx
        });
        sender.subscribe()
    }

    /// Subscribe to real-time pushed trade events.
    pub fn subscribe_trade(&self) -> broadcast::Receiver<TradeEvent> {
        self.trades.subscribe()
    }

    /// Subscribe to real-time pushed depth-of-market book events for a symbol.
    pub fn subscribe_book(&self, symbol: &str) -> broadcast::Receiver<BookEvent> {
        let mut map = self.books.lock().unwrap();
        let sender = map.entry(symbol.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(self.buffer_capacity);
            tx
        });
        sender.subscribe()
    }

    /// Dispatch an incoming tick to all subscribers of that symbol.
    pub fn dispatch_tick(&self, tick: Tick) {
        let map = self.ticks.lock().unwrap();
        if let Some(tx) = map.get(&tick.symbol) {
            let _ = tx.send(tick);
        }
    }

    /// Dispatch an incoming trade transaction event to all trade subscribers.
    pub fn dispatch_trade(&self, trade: TradeEvent) {
        let _ = self.trades.send(trade);
    }

    /// Dispatch an incoming book depth event to all subscribers of that symbol.
    pub fn dispatch_book(&self, book: BookEvent) {
        let map = self.books.lock().unwrap();
        if let Some(tx) = map.get(&book.symbol) {
            let _ = tx.send(book);
        }
    }

    /// Returns the symbols currently tracked in the tick subscription table.
    pub fn active_tick_symbols(&self) -> Vec<String> {
        self.ticks.lock().unwrap().keys().cloned().collect()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_TICK_BUFFER)
    }
}

/// Active subscription to real-time market data ticks for a symbol (protocol v5+).
pub struct TickSubscription {
    symbol: String,
    mode: StreamMode,
    rx: broadcast::Receiver<Tick>,
    dropped_ticks: u64,
}

impl TickSubscription {
    pub fn new(symbol: impl Into<String>, mode: StreamMode, rx: broadcast::Receiver<Tick>) -> Self {
        Self {
            symbol: symbol.into(),
            mode,
            rx,
            dropped_ticks: 0,
        }
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn mode(&self) -> StreamMode {
        self.mode
    }

    pub fn dropped_ticks(&self) -> u64 {
        self.dropped_ticks
    }

    /// Asynchronously await and receive the next tick from the push feed.
    ///
    /// Under `StreamMode::Latest`, if the consumer lags and intermediate quotes are dropped,
    /// this method updates the drop counter, logs a warning, and yields the freshest quote
    /// rather than returning an error.
    ///
    /// Returns `None` if the sender was closed.
    pub async fn recv(&mut self) -> Option<Tick> {
        loop {
            match self.rx.recv().await {
                Ok(tick) => return Some(tick),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    self.dropped_ticks += skipped;
                    if self.mode == StreamMode::Latest {
                        if self.dropped_ticks % 100 <= skipped {
                            warn!(
                                symbol = %self.symbol,
                                dropped_ticks = self.dropped_ticks,
                                skipped,
                                "Tick subscription lagged: dropped stale quotes to maintain low latency"
                            );
                        }
                    } else {
                        warn!(
                            symbol = %self.symbol,
                            dropped_ticks = self.dropped_ticks,
                            skipped,
                            "Tick subscription (Lossless) lagged: consumer fell behind broadcast buffer"
                        );
                    }
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Asynchronously receive the next tick, returning `Err(RecvError::Lagged)` if consumer lag occurred.
    ///
    /// This allows recorder / auditing consumers to explicitly detect quote gaps.
    pub async fn recv_checked(&mut self) -> Result<Tick, broadcast::error::RecvError> {
        match self.rx.recv().await {
            Ok(tick) => Ok(tick),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                self.dropped_ticks += skipped;
                Err(broadcast::error::RecvError::Lagged(skipped))
            }
            Err(e) => Err(e),
        }
    }

    /// Attempt to receive a tick without waiting.
    ///
    /// In [`StreamMode::Latest`], if lag occurred, it automatically discards the stale ticks
    /// and attempts to fetch the newest quote.
    /// In [`StreamMode::Lossless`], it returns [`broadcast::error::TryRecvError::Lagged`].
    pub fn try_recv(&mut self) -> Result<Tick, broadcast::error::TryRecvError> {
        match self.rx.try_recv() {
            Ok(tick) => Ok(tick),
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                self.dropped_ticks += skipped;
                if self.mode == StreamMode::Lossless {
                    Err(broadcast::error::TryRecvError::Lagged(skipped))
                } else {
                    self.rx.try_recv()
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Create a new independent receiver subscribed to the same symbol's broadcast feed.
    pub fn resubscribe(&self) -> Self {
        Self {
            symbol: self.symbol.clone(),
            mode: self.mode,
            rx: self.rx.resubscribe(),
            dropped_ticks: 0,
        }
    }
}

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
///
/// On protocol v5+ bridges, this automatically uses event-driven push subscriptions from MT5
/// with zero polling interval. On older bridges or backends without push support, it falls back
/// to periodic polling.
pub fn stream_ticks_with_config(
    client: Arc<Mt5Client>,
    symbol: &str,
    config: StreamConfig,
) -> mpsc::Receiver<Tick> {
    let (tx, rx) = mpsc::channel(config.buffer_size.max(16));
    let sym_owned = symbol.to_string();

    let stream_mode = match config.backpressure {
        BackpressurePolicy::DropLatest => StreamMode::Latest,
        BackpressurePolicy::Block => StreamMode::Lossless,
    };

    // If client supports push subscription, use event-driven push delivery
    if let Ok(mut sub) = client.subscribe_ticks_with_mode(symbol, stream_mode) {
        let tx_push = tx.clone();
        let sym_push = sym_owned.clone();
        tokio::spawn(async move {
            debug!(symbol = %sym_push, "Push-based tick stream started");
            let mut dropped_ticks: u64 = 0;
            while let Some(tick) = sub.recv().await {
                match config.backpressure {
                    BackpressurePolicy::DropLatest => match offer_drop_newest(&tx_push, tick) {
                        Offer::Delivered => {}
                        Offer::DroppedNewest => {
                            dropped_ticks += 1;
                            if dropped_ticks % 100 == 1 {
                                warn!(
                                    symbol = %sym_push,
                                    dropped_ticks,
                                    "Tick stream buffer full: discarded newest tick"
                                );
                            }
                        }
                        Offer::ReceiverClosed => break,
                    },
                    BackpressurePolicy::Block => {
                        if tx_push.send(tick).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        return rx;
    }

    tokio::spawn(async move {
        debug!(symbol = %sym_owned, "Polling fallback tick stream started");

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
                                BackpressurePolicy::DropLatest => {
                                    match offer_drop_newest(&tx, tick) {
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
                                    }
                                }
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

        let received: Vec<i64> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|t| t.time_msc)
            .collect();
        assert_eq!(
            received,
            vec![1, 2, 3, 4],
            "consumer sees the stale prefix, never 5 or 6"
        );

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

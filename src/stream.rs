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
    bars: Mutex<HashMap<(String, i32), broadcast::Sender<Bar>>>,
    buffer_capacity: usize,
}

impl EventBus {
    pub fn new(buffer_capacity: usize) -> Self {
        let (trades, _) = broadcast::channel(buffer_capacity.max(64));
        Self {
            ticks: Mutex::new(HashMap::new()),
            trades,
            books: Mutex::new(HashMap::new()),
            bars: Mutex::new(HashMap::new()),
            buffer_capacity: buffer_capacity.max(64),
        }
    }

    /// Subscribe to real-time pushed ticks for a symbol.
    pub fn subscribe_ticks(&self, symbol: &str) -> broadcast::Receiver<Tick> {
        let mut map = self
            .ticks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let mut map = self
            .books
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sender = map.entry(symbol.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(self.buffer_capacity);
            tx
        });
        sender.subscribe()
    }

    /// Subscribe to real-time pushed OHLCV bars for a symbol and timeframe.
    pub fn subscribe_bars(&self, symbol: &str, timeframe: i32) -> broadcast::Receiver<Bar> {
        let mut map = self
            .bars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sender = map
            .entry((symbol.to_string(), timeframe))
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.buffer_capacity);
                tx
            });
        sender.subscribe()
    }

    /// Dispatch an incoming tick to all subscribers of that symbol.
    pub fn dispatch_tick(&self, tick: Tick) {
        let map = self
            .ticks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let map = self
            .books
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(tx) = map.get(&book.symbol) {
            let _ = tx.send(book);
        }
    }

    /// Dispatch an incoming OHLCV bar event to all subscribers of that symbol and timeframe.
    pub fn dispatch_bar(&self, symbol: &str, timeframe: i32, bar: Bar) {
        let map = self
            .bars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(tx) = map.get(&(symbol.to_string(), timeframe)) {
            let _ = tx.send(bar);
        }
    }

    /// Returns the symbols currently tracked in the tick subscription table.
    pub fn active_tick_symbols(&self) -> Vec<String> {
        self.ticks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Returns the symbols currently tracked in the book subscription table.
    pub fn active_book_symbols(&self) -> Vec<String> {
        self.books
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Remove the book sender for `symbol` when no subscribers remain.
    ///
    /// Callers may invoke this after the last [`BookSubscription`] for a symbol is dropped
    /// so the internal sender map does not grow indefinitely.
    pub fn prune_book(&self, symbol: &str) {
        let mut map = self
            .books
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(tx) = map.get(symbol) {
            if tx.receiver_count() == 0 {
                map.remove(symbol);
            }
        }
    }

    /// Remove the tick sender for `symbol` when no subscribers remain.
    ///
    /// Callers may invoke this after the EA acknowledges an unsubscribe so the
    /// internal tick sender map does not grow indefinitely.
    pub fn remove_tick_sender(&self, symbol: &str) {
        let mut map = self
            .ticks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if map.get(symbol).is_some_and(|tx| tx.receiver_count() == 0) {
            map.remove(symbol);
        }
    }

    /// Periodic sweep: remove tick, book, and bar senders that have no remaining receivers.
    ///
    /// Intended for scheduled maintenance so that symbols whose subscriptions have all
    /// been dropped do not linger in the internal maps forever. Each map is drained
    /// independently under its own lock, so a long-held lock on one stream kind does not
    /// block sweeping the others.
    pub fn prune(&self) {
        let mut ticks = self
            .ticks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ticks.retain(|_, tx| tx.receiver_count() > 0);
        drop(ticks);

        let mut books = self
            .books
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        books.retain(|_, tx| tx.receiver_count() > 0);
        drop(books);

        let mut bars = self
            .bars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        bars.retain(|_, tx| tx.receiver_count() > 0);
        drop(bars);
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
    seq: SequenceTracker,
}

impl TickSubscription {
    pub fn new(symbol: impl Into<String>, mode: StreamMode, rx: broadcast::Receiver<Tick>) -> Self {
        Self {
            symbol: symbol.into(),
            mode,
            rx,
            dropped_ticks: 0,
            seq: SequenceTracker::default(),
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

    /// Total upstream sequence gaps observed on this subscription.
    pub fn gaps(&self) -> u64 {
        self.seq.gaps()
    }

    /// Last upstream sequence number received, or `None` if no sequenced event arrived yet.
    pub fn last_seq(&self) -> Option<u64> {
        self.seq.last_seq()
    }

    /// Asynchronously await and receive the next tick from the push feed.
    ///
    /// Under `StreamMode::Latest`, if the consumer lags and intermediate quotes are dropped,
    /// this method updates the drop counter, logs a warning, and drains the remaining buffer
    /// to yield the freshest available quote rather than returning stale retained quotes.
    ///
    /// Returns `None` if the sender was closed.
    pub async fn recv(&mut self) -> Option<Tick> {
        loop {
            match self.rx.recv().await {
                Ok(tick) => {
                    self.seq.observe(tick.event_seq);
                    return Some(tick);
                }
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
                        // Drain to the freshest quote in the buffer so lagging consumers
                        // do not process old retained backlog.
                        let mut latest = None;
                        let mut drained: u64 = 0;
                        loop {
                            match self.rx.try_recv() {
                                Ok(tick) => {
                                    if latest.is_some() {
                                        drained += 1;
                                    }
                                    latest = Some(tick);
                                }
                                Err(broadcast::error::TryRecvError::Lagged(more_skipped)) => {
                                    self.dropped_ticks += more_skipped;
                                    continue;
                                }
                                Err(broadcast::error::TryRecvError::Empty) => break,
                                Err(broadcast::error::TryRecvError::Closed) => break,
                            }
                        }
                        self.dropped_ticks += drained;
                        if let Some(tick) = latest {
                            self.seq.observe(tick.event_seq);
                            return Some(tick);
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
            Ok(tick) => {
                self.seq.observe(tick.event_seq);
                Ok(tick)
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                self.dropped_ticks += skipped;
                Err(broadcast::error::RecvError::Lagged(skipped))
            }
            Err(e) => Err(e),
        }
    }

    /// Attempt to receive a tick without waiting.
    ///
    /// In [`StreamMode::Latest`], if lag occurred, it discards the stale ticks, drains to the
    /// newest available quote, and returns it.
    /// In [`StreamMode::Lossless`], it returns [`broadcast::error::TryRecvError::Lagged`].
    pub fn try_recv(&mut self) -> Result<Tick, broadcast::error::TryRecvError> {
        match self.rx.try_recv() {
            Ok(tick) => {
                self.seq.observe(tick.event_seq);
                Ok(tick)
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                self.dropped_ticks += skipped;
                if self.mode == StreamMode::Lossless {
                    Err(broadcast::error::TryRecvError::Lagged(skipped))
                } else {
                    let mut latest = None;
                    let mut drained: u64 = 0;
                    loop {
                        match self.rx.try_recv() {
                            Ok(t) => {
                                if latest.is_some() {
                                    drained += 1;
                                }
                                latest = Some(t);
                            }
                            Err(broadcast::error::TryRecvError::Lagged(more)) => {
                                self.dropped_ticks += more;
                                continue;
                            }
                            Err(broadcast::error::TryRecvError::Empty) => break,
                            Err(broadcast::error::TryRecvError::Closed) => break,
                        }
                    }
                    self.dropped_ticks += drained;
                    if let Some(t) = latest {
                        self.seq.observe(t.event_seq);
                        Ok(t)
                    } else {
                        Err(broadcast::error::TryRecvError::Empty)
                    }
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
            seq: SequenceTracker::default(),
        }
    }
}

/// Active subscription to pushed depth-of-market book events for a symbol (protocol v5+).
///
/// Wraps a broadcast receiver with per-subscription sequence gap detection. Use
/// [`recv_checked`](BookSubscription::recv_checked) to receive events and detect upstream drops.
pub struct BookSubscription {
    symbol: String,
    rx: Option<broadcast::Receiver<BookEvent>>,
    seq: SequenceTracker,
    dropped: u64,
    pending: Option<BookEvent>,
    bus: Option<std::sync::Arc<EventBus>>,
}

impl BookSubscription {
    /// Construct a subscription from a raw broadcast receiver.
    pub fn new(symbol: impl Into<String>, rx: broadcast::Receiver<BookEvent>) -> Self {
        Self {
            symbol: symbol.into(),
            rx: Some(rx),
            seq: SequenceTracker::default(),
            dropped: 0,
            pending: None,
            bus: None,
        }
    }

    /// Construct a subscription from a raw broadcast receiver with an attached [`EventBus`].
    ///
    /// When the subscription is dropped, it will automatically prune idle book senders
    /// from the bus if no other subscribers remain for `symbol`.
    pub fn with_bus(
        symbol: impl Into<String>,
        rx: broadcast::Receiver<BookEvent>,
        bus: std::sync::Arc<EventBus>,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            rx: Some(rx),
            seq: SequenceTracker::default(),
            dropped: 0,
            pending: None,
            bus: Some(bus),
        }
    }

    /// Symbol this subscription tracks.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Total events dropped by broadcast buffer overflow (consumer lag).
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Total upstream sequence gaps observed on this subscription.
    pub fn gaps(&self) -> u64 {
        self.seq.gaps()
    }

    /// Total out-of-order events observed on this subscription.
    pub fn reordered(&self) -> u64 {
        self.seq.reordered()
    }

    /// Last upstream sequence number received, or `None` if no sequenced event arrived.
    pub fn last_seq(&self) -> Option<u64> {
        self.seq.last_seq()
    }

    /// Asynchronously receive the next book event.
    ///
    /// Consumer buffer-overflow lag is tracked internally and logged.
    /// Returns `None` when the sender channel is closed.
    pub async fn recv(&mut self) -> Option<BookEvent> {
        if let Some(ev) = self.pending.take() {
            return Some(ev);
        }
        let rx = self.rx.as_mut()?;
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    self.seq.observe(ev.event_seq);
                    return Some(ev);
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    self.dropped += skipped;
                    warn!(
                        symbol = %self.symbol,
                        dropped = self.dropped,
                        skipped,
                        "Book subscription lagged: consumer fell behind broadcast buffer"
                    );
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Asynchronously receive the next book event with sequence gap detection.
    ///
    /// - Returns `Ok(Some(ev))` when delivered cleanly.
    /// - Returns `Err(StreamError::Gap { expected, got })` when an upstream sequence gap is detected;
    ///   the event is retained and will be yielded on the subsequent call.
    /// - Returns `Err(StreamError::Lagged(skipped))` on broadcast buffer overrun.
    /// - Returns `Ok(None)` when the stream was closed by upstream.
    pub async fn recv_checked(&mut self) -> Result<Option<BookEvent>, StreamError> {
        if let Some(ev) = self.pending.take() {
            return Ok(Some(ev));
        }
        let rx = self.rx.as_mut().ok_or(StreamError::Closed)?;
        match rx.recv().await {
            Ok(ev) => {
                if let Some((first_missing, _)) = self.seq.observe(ev.event_seq) {
                    self.pending = Some(ev.clone());
                    Err(StreamError::Gap {
                        expected: first_missing,
                        got: ev.event_seq,
                    })
                } else {
                    Ok(Some(ev))
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                self.dropped += skipped;
                Err(StreamError::Lagged(skipped))
            }
            Err(broadcast::error::RecvError::Closed) => Ok(None),
        }
    }
}

impl Drop for BookSubscription {
    fn drop(&mut self) {
        self.rx = None;
        if let Some(bus) = &self.bus {
            bus.prune_book(&self.symbol);
        }
    }
}

/// Tracks monotonic per-stream sequence numbers and reports gaps.
///
/// Sequence zero is treated as "no sequence" and is always ignored.
/// Out-of-order arrivals (sequence regressed) are counted as reorders; they
/// do not advance `last` and are not counted as gaps.
#[derive(Debug, Default)]
pub struct SequenceTracker {
    last: Option<u64>,
    gaps: u64,
    reordered: u64,
}

impl SequenceTracker {
    /// Record a new sequence number.
    ///
    /// Returns `Some((first_missing, last_missing))` when a gap is detected, i.e.
    /// the observed sequence jumped forward by more than one from the previous value.
    /// Returns `None` when the event is contiguous, the first event, or seq is zero.
    pub fn observe(&mut self, seq: u64) -> Option<(u64, u64)> {
        if seq == 0 {
            return None;
        }
        let gap = match self.last {
            Some(prev) if seq == prev + 1 => None,
            Some(prev) if seq > prev + 1 => Some((prev + 1, seq - 1)),
            Some(prev) if seq <= prev => {
                self.reordered += 1;
                return None;
            }
            Some(_) => None,
            None => None,
        };
        self.last = Some(seq);
        if gap.is_some() {
            self.gaps += 1;
        }
        gap
    }

    /// Total number of gap events observed since creation.
    pub fn gaps(&self) -> u64 {
        self.gaps
    }

    /// Total number of out-of-order (reordered) events observed since creation.
    pub fn reordered(&self) -> u64 {
        self.reordered
    }

    /// Last sequence number seen, or `None` if no non-zero event has been observed.
    pub fn last_seq(&self) -> Option<u64> {
        self.last
    }
}

const DEFAULT_TICK_BUFFER: usize = 1024;
const DEFAULT_BAR_BUFFER: usize = 256;

use crate::constants::DUP_PRICE_THRESHOLD;

/// Maximum consecutive poll errors allowed before a streaming task aborts and closes the channel.
pub const MAX_CONSECUTIVE_STREAM_ERRORS: u32 = 10;

/// Error reasons that cause a streaming task to terminate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// The upstream subscription closed or disconnected.
    #[error("Stream closed by upstream feed")]
    Closed,

    /// Consumer lagged behind stream buffer.
    #[error("Stream consumer lagged by {0} events")]
    Lagged(u64),

    /// Upstream event sequence jumped forward, indicating dropped events between
    /// `expected` and `got` (inclusive range of missing sequence numbers).
    #[error("Stream gap detected: expected seq {expected}, got {got}")]
    Gap { expected: u64, got: u64 },

    /// Terminated after exceeding maximum consecutive polling errors.
    #[error("Stream exceeded maximum consecutive errors ({count}): {last_error}")]
    MaxConsecutiveErrors { count: u32, last_error: String },

    /// Background polling task joined with error.
    #[error("Stream task joined with error: {0}")]
    TaskJoinError(String),
}

/// Calculates the closed (end) timestamp of a bar given its timeframe and open timestamp.
pub fn bar_end_time(timeframe: Timeframe, open_time: i64) -> i64 {
    if timeframe == Timeframe::MN1 {
        if let Some(dt) = chrono::DateTime::from_timestamp(open_time, 0) {
            use chrono::Datelike;
            let year = dt.year();
            let month = dt.month();
            let (next_year, next_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            if let Some(next_dt) = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
                .and_then(|d| d.and_hms_opt(0, 0, 0))
            {
                return next_dt.and_utc().timestamp();
            }
        }
    }
    open_time + timeframe.seconds()
}

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

fn offer_drop_newest(tx: &mpsc::Sender<Result<Tick, StreamError>>, tick: Tick) -> Offer {
    match tx.try_send(Ok(tick)) {
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
) -> mpsc::Receiver<Result<Tick, StreamError>> {
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
                        Offer::ReceiverClosed => return,
                    },
                    BackpressurePolicy::Block => {
                        if tx_push.send(Ok(tick)).await.is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = tx_push.send(Err(StreamError::Closed)).await;
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
                                    if tx.send(Ok(tick)).await.is_err() {
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
                        let _ = tx
                            .send(Err(StreamError::MaxConsecutiveErrors {
                                count: consecutive_errors,
                                last_error: e.to_string(),
                            }))
                            .await;
                        break;
                    }
                }
                Err(e) => {
                    error!(symbol = %sym_owned, error = %e, "Tick stream task joined with error");
                    let _ = tx
                        .send(Err(StreamError::TaskJoinError(e.to_string())))
                        .await;
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
) -> mpsc::Receiver<Result<Tick, StreamError>> {
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
pub fn stream_bars(
    client: Arc<Mt5Client>,
    symbol: &str,
    timeframe: Timeframe,
    poll_interval: Duration,
    include_incomplete: bool,
) -> mpsc::Receiver<Result<Bar, StreamError>> {
    let (tx, rx) = mpsc::channel(DEFAULT_BAR_BUFFER);
    let sym_owned = symbol.to_string();

    tokio::spawn(async move {
        debug!(symbol = %sym_owned, timeframe = %timeframe, "Bar stream started");

        let mut last_closed_bar_time: Option<i64> = None;
        let mut last_current: Option<Bar> = None;
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
                    if !rates.is_empty() {
                        if let Some(prev_closed_time) = last_closed_bar_time {
                            for rate in &rates {
                                if rate.time > prev_closed_time
                                    && bar_end_time(timeframe, rate.time) <= now
                                {
                                    last_closed_bar_time = Some(rate.time);
                                    last_current = None;
                                    let bar = Bar::from(*rate);
                                    if tx.send(Ok(bar)).await.is_err() {
                                        debug!(symbol = %sym_owned, "Bar stream receiver dropped; shutting down");
                                        return;
                                    }
                                }
                            }
                        } else {
                            let newest_closed = rates
                                .iter()
                                .filter(|r| bar_end_time(timeframe, r.time) <= now)
                                .map(|r| r.time)
                                .max();
                            if let Some(t) = newest_closed {
                                last_closed_bar_time = Some(t);
                            }
                        }
                    }

                    if include_incomplete {
                        if let Some(current_rate) = rates.last().copied() {
                            if bar_end_time(timeframe, current_rate.time) > now {
                                let current = Bar::from(current_rate);
                                if last_current != Some(current) {
                                    last_current = Some(current);
                                    if tx.send(Ok(current)).await.is_err() {
                                        debug!(symbol = %sym_owned, "Bar stream receiver dropped; shutting down");
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
                        "Bar stream poll error"
                    );
                    if consecutive_errors >= MAX_CONSECUTIVE_STREAM_ERRORS {
                        error!(
                            symbol = %sym_owned,
                            consecutive_errors,
                            "Bar stream exceeded maximum consecutive errors; terminating stream"
                        );
                        let _ = tx
                            .send(Err(StreamError::MaxConsecutiveErrors {
                                count: consecutive_errors,
                                last_error: e.to_string(),
                            }))
                            .await;
                        break;
                    }
                }
                Err(e) => {
                    error!(symbol = %sym_owned, error = %e, "Bar stream task joined with error");
                    let _ = tx
                        .send(Err(StreamError::TaskJoinError(e.to_string())))
                        .await;
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
            event_seq: 0,
        }
    }

    /// Pins the documented `DropLatest` semantics: a full buffer discards the NEWEST tick and the
    /// consumer receives the OLDEST backlog first (not "latest value wins").
    #[test]
    fn drop_latest_discards_the_newest_tick_and_keeps_the_stale_backlog() {
        let (tx, mut rx) = mpsc::channel::<Result<Tick, StreamError>>(4);
        for n in 1..=4 {
            assert_eq!(offer_drop_newest(&tx, tick(n)), Offer::Delivered);
        }
        // buffer full: 5 and 6 are the newest quotes and are the ones that get discarded
        assert_eq!(offer_drop_newest(&tx, tick(5)), Offer::DroppedNewest);
        assert_eq!(offer_drop_newest(&tx, tick(6)), Offer::DroppedNewest);

        let received: Vec<i64> = std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|r| r.ok())
            .map(|t| t.time_msc)
            .collect();
        assert_eq!(
            received,
            vec![1, 2, 3, 4],
            "consumer sees the stale prefix, never 5 or 6"
        );

        // once drained the feed resumes, leaving a gap (5, 6) as the only in-band signal
        assert_eq!(offer_drop_newest(&tx, tick(7)), Offer::Delivered);
        assert_eq!(rx.try_recv().unwrap().unwrap().time_msc, 7);
    }

    #[test]
    fn drop_latest_reports_a_closed_receiver() {
        let (tx, rx) = mpsc::channel::<Result<Tick, StreamError>>(4);
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

    #[test]
    fn bar_end_time_calculates_correct_spans() {
        let t0 = 1704067200; // 2024-01-01 00:00:00 UTC
        assert_eq!(bar_end_time(Timeframe::M1, t0), t0 + 60);
        assert_eq!(bar_end_time(Timeframe::M5, t0), t0 + 300);
        assert_eq!(bar_end_time(Timeframe::H1, t0), t0 + 3600);
        assert_eq!(bar_end_time(Timeframe::D1, t0), t0 + 86400);
        assert_eq!(bar_end_time(Timeframe::W1, t0), t0 + 604800);

        // MN1 in leap year 2024: Feb 1 to Mar 1 is 29 days
        let feb_2024 = 1706745600; // 2024-02-01 00:00:00 UTC
        let mar_2024 = 1709251200; // 2024-03-01 00:00:00 UTC
        assert_eq!(bar_end_time(Timeframe::MN1, feb_2024), mar_2024);
        assert_eq!(mar_2024 - feb_2024, 29 * 86400);

        // MN1 Dec to Jan transition
        let dec_2024 = 1733011200; // 2024-12-01 00:00:00 UTC
        let jan_2025 = 1735689600; // 2025-01-01 00:00:00 UTC
        assert_eq!(bar_end_time(Timeframe::MN1, dec_2024), jan_2025);
        assert_eq!(jan_2025 - dec_2024, 31 * 86400);
    }

    #[tokio::test]
    async fn tick_subscription_latest_mode_drains_to_freshest_on_lag() {
        let bus = EventBus::new(64);
        let rx = bus.subscribe_ticks("EURUSD");
        let mut sub = TickSubscription::new("EURUSD", StreamMode::Latest, rx);

        // Send 100 ticks to overflow the 64-capacity channel
        for i in 1..=100 {
            bus.dispatch_tick(tick(i));
        }

        // recv() must yield the freshest quote (tick 100) and track all dropped ticks
        let latest = sub.recv().await.expect("tick expected");
        assert_eq!(latest.time_msc, 100);
        assert_eq!(sub.dropped_ticks(), 99);
    }

    #[test]
    fn tick_subscription_latest_mode_try_recv_drains_on_lag() {
        let bus = EventBus::new(64);
        let rx = bus.subscribe_ticks("EURUSD");
        let mut sub = TickSubscription::new("EURUSD", StreamMode::Latest, rx);

        for i in 1..=100 {
            bus.dispatch_tick(tick(i));
        }

        let latest = sub.try_recv().expect("tick expected");
        assert_eq!(latest.time_msc, 100);
        assert_eq!(sub.dropped_ticks(), 99);
    }
}

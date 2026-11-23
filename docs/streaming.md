# Streaming APIs

`mt5-bridge` provides two categories of real-time streaming:

1. **Push Streaming** — event-driven, fed directly from MT5's `OnTick()`, `OnBookEvent()`, `OnTradeTransaction()`, and bar closing events (Protocol v5+).
2. **Polling Streams** — background tasks that poll `symbol_tick()` / `copy_bars()` at a configurable interval, with deduplication and backpressure.

Both are built on Tokio async broadcast & mpsc channels (enabled via the default `async` feature).

---

## Real-Time Push Streaming

Subscribe once, receive events as they arrive from MT5:

```rust,no_run
use mt5_bridge::{Mt5Client, StreamMode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Mt5Client::connect(12345678, "password", "Broker-Demo")?;

    // Option A: Real-time event push subscription directly from MT5 OnTick()
    // StreamMode::Latest (default): Execution stream, drops stale quotes if consumer lags
    // StreamMode::Lossless: Recorder stream, surfaces RecvError::Lagged if consumer falls behind
    let mut sub = client.subscribe_ticks_with_mode("EURUSD", StreamMode::Latest)?;

    let mut count = 0;
    while let Some(tick) = sub.recv().await {
        println!(
            "Push Tick -> Time: {} | Bid: {:.5} | Ask: {:.5} | Spread: {:.5} (dropped: {})",
            tick.time, tick.bid, tick.ask, tick.spread(), sub.dropped_ticks()
        );
        count += 1;
        if count >= 10 {
            client.unsubscribe_ticks("EURUSD")?;
            break;
        }
    }

    Ok(())
}
```

### Push Stream Methods

| Method | Signature | Description |
| :--- | :--- | :--- |
| `subscribe_ticks` | `pub fn subscribe_ticks(&self, symbol: &str) -> Result<TickSubscription>` | Subscribe to push tick events using default `StreamMode::Latest`. |
| `subscribe_ticks_with_mode` | `pub fn subscribe_ticks_with_mode(&self, symbol: &str, mode: StreamMode) -> Result<TickSubscription>` | Subscribe with explicit mode: `Lossless` or `Latest`. |
| `unsubscribe_ticks` | `pub fn unsubscribe_ticks(&self, symbol: &str) -> Result<()>` | Unsubscribe from push tick quotes. |
| `subscribe_book` | `pub fn subscribe_book(&self, symbol: &str) -> Result<BookSubscription>` | Subscribe to real-time Level II Market Depth updates. |
| `unsubscribe_book` | `pub fn unsubscribe_book(&self, symbol: &str) -> Result<()>` | Unsubscribe from Level II depth stream. |
| `subscribe_trade` | `pub fn subscribe_trade(&self) -> Result<TradeSubscription>` | Subscribe to real-time broker trade transactions with order comment attribution. |
| `unsubscribe_trade` | `pub fn unsubscribe_trade(&self) -> Result<()>` | Unsubscribe from broker trade transactions stream. |
| `subscribe_bars` | `pub fn subscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<tokio::sync::broadcast::Receiver<Bar>>` | Subscribe to real-time closed-bar notifications. |
| `unsubscribe_bars` | `pub fn unsubscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<()>` | Unsubscribe from closed-bar stream. |

---

## Polling-Based Streams (Fallback & Candle Aggregation)

```rust,no_run
use mt5_bridge::{stream_bars, Mt5Client, Timeframe};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Mt5Client::connect(12345678, "password", "Broker-Demo")?);

    // Listen for newly closed 1-minute bars
    let mut rx = stream_bars(client, "EURUSD", Timeframe::M1, Duration::from_millis(500), false);

    while let Some(Ok(bar)) = rx.recv().await {
        println!(
            "Closed Candle -> Time: {} | Open: {:.5} | Close: {:.5} | Range: {:.5} | Vol: {:.0}",
            bar.time, bar.open, bar.close, bar.range(), bar.volume
        );
    }

    Ok(())
}
```

### Polling Stream Functions

| Function | Signature | Description |
| :--- | :--- | :--- |
| `stream_ticks` | `pub fn stream_ticks(client: Arc<Mt5Client>, symbol: &str, poll_interval: Duration) -> mpsc::Receiver<Tick>` | Spawns background task polling `symbol_tick()`, deduplicates identical ticks. |
| `stream_ticks_with_config` | `pub fn stream_ticks_with_config(client, symbol, config: StreamConfig) -> mpsc::Receiver<Tick>` | Tick streaming with explicit buffer size, interval, and backpressure policy. |
| `stream_bars` | `pub fn stream_bars(client, symbol, timeframe, poll_interval, include_incomplete) -> mpsc::Receiver<Result<Bar, StreamError>>` | Emits completed `Bar` on candle close. Extended lookback for calendar intervals (W1, MN1). |

---

## Stream Backpressure & Loss Semantics

### `BackpressurePolicy::DropLatest` (default)

When the receiver channel buffer is full, newly arrived quotes are discarded (`try_send` fails). This prevents queue lag accumulation and guarantees low latency for execution algorithms: consumers always process the latest sampled market state rather than a stale backlog. Note that the buffer retains previously queued quotes and discards the newest incoming quote until the consumer catches up.

### `BackpressurePolicy::Block`

When the receiver buffer is full, the background poller asynchronously awaits channel capacity. This guarantees zero message loss (every sampled quote is delivered), suitable for telemetry, logging, and data-recording pipelines where completeness is prioritized over execution latency.

---

## Python Streaming (Task 5)

The Python `AsyncClient` wraps the same event delivery system:

```python
import asyncio
import mt5_bridge_py as mb

async def main():
    client = mb.AsyncClient.connect_with_secret("test")

    # Register a callback invoked by the DLL on each streamed event
    def on_event(event_type: int, payload: bytes):
        print(f"Event type={event_type}, payload_len={len(payload)}")

    client.register_event_callback(on_event)

    # Consume events via async iterator
    count = 0
    async for event in client.events():
        print(f"PyEvent: type={event.event_type}, payload={event.payload[:20]}")
        count += 1
        if count >= 5:
            break

    # Queue observability
    print(f"Dropped: {client.events_dropped_total()}")
    print(f"Orphans: {client.get_orphan_count()}")

    # Single poll with timeout
    ev = client.poll_event(100)
    if ev is not None:
        print(f"Polled: type={ev.event_type}")

    await client.shutdown()

asyncio.run(main())
```

| Python Method | Description |
| :--- | :--- |
| `register_event_callback(cb)` | Register Python callable `(event_type: int, payload: bytes) -> None` invoked by the DLL on each event |
| `poll_event(timeout_ms)` | Poll for next event; `None` on timeout |
| `events_dropped_total()` | DLL ring-buffer dropped-event count (`int`) |
| `get_orphan_count()` | Orphaned events held by DLL queue (`int`) |
| `async for event in client.events():` | Async event stream yielding `PyEvent` |

> **One client per process:** The DLL enforces a single connection per process. Sync and async Python test suites must run in separate Wine Python processes.

## Related Documents

- [API Reference](api-reference.md) — `Mt5Client` streaming method signatures
- [Benchmarks](benchmarks.md) — push dispatch latency, fanout, jitter stats
- [Architecture](architecture.md) — event processing model, queue overflow & observability
- [Protocol Specification](protocol.md) — wire framing for tick/trade/book/bar events

# Rust API Reference

Comprehensive reference for public structs, enums, methods, and functions in `mt5-bridge` (version 0.5.0).

---

## `Mt5Client`

`Mt5Client` is the primary thread-safe client managing dynamic DLL loading, named pipe IPC, market data queries, and trade execution.

### Connection & Lifecycle

| Method | Signature | Description |
| :--- | :--- | :--- |
| `connect` | `pub fn connect(login: i64, password: &str, server: &str) -> Result<Self>` | Connects to MT5. Resolves `mt5_bridge.dll` via `MT5_DLL_PATH` env var, or adjacent to executable / CWD. |
| `connect_with_dll` | `pub fn connect_with_dll(dll_path: impl AsRef<Path>, login: i64, password: &str, server: &str) -> Result<Self>` | Connects by loading the bridge DLL from an explicit file path. |
| `connect_with_secret` | `pub fn connect_with_secret(secret: &str) -> Result<Self>` | Connects using only the pipe secret token (recommended; keeps broker credentials off IPC). |
| `process_connection_count` | `pub fn process_connection_count() -> usize` | Returns the number of live clients in this process (0 or 1). Diagnostics / health checks. |
| `set_symbol_cache_ttl` | `pub fn set_symbol_cache_ttl(&mut self, ttl: Duration)` | Overrides in-memory cache TTL for `symbol_info` lookups (default: 60 seconds). |
| `shutdown` | `pub fn shutdown(&self) -> Result<()>` | Gracefully disconnects from the MT5 pipe server. Automatically called on `Drop`. |

> **One client per process.** A second `connect*` call while a client is alive returns `Mt5Error::AlreadyConnected`. Run one process per MT5 terminal.

### Bridge Diagnostics & Observability

| Method | Signature | Description |
| :--- | :--- | :--- |
| `ping` | `pub fn ping(&self) -> Result<PingResult>` | Heartbeat returning server time, protocol version, and RTT. |
| `stats` | `pub fn stats(&self) -> Result<BridgeStats>` | Bridge uptime, request/drop counters, queue depths, last error, protocol version. Returns `UnsupportedFeature` if the DLL lacks the `Stats` export. |
| `process_connection_count` | `pub fn process_connection_count() -> usize` | Returns the number of live clients in this process (0 or 1). Diagnostics / health checks. |

### Account & Symbol Data

| Method | Signature | Description |
| :--- | :--- | :--- |
| `account_info` | `pub fn account_info(&self) -> Result<AccountInfo>` | Real-time balance, equity, margin, free margin in deposit currency. |
| `symbol_info` | `pub fn symbol_info(&self, symbol: &str) -> Result<SymbolInfo>` | Contract specifications (point size, tick size, tick value, lot step, min/max lots, spread, digits) with thread-safe caching. |
| `symbol_tick` | `pub fn symbol_tick(&self, symbol: &str) -> Result<Tick>` | Latest quote tick (bid, ask, last, volume, flags). |
| `symbols_total` | `pub fn symbols_total(&self) -> Result<u32>` | Total number of symbols available in the terminal (`CMD_SYMBOLS_TOTAL`). |
| `symbols_page` | `pub fn symbols_page(&self, page: u32, page_size: u32) -> Result<Vec<SymbolInfo>>` | Paged symbol list (`CMD_SYMBOLS_GET`). |
| `symbols` | `pub fn symbols(&self) -> Result<Vec<SymbolInfo>>` | Full symbol list, paging internally. |
| `symbols_filtered` | `pub fn symbols_filtered(&self, ...) -> Result<Vec<SymbolInfo>>` | Symbol list with optional filters. |
| `symbol_select` | `pub fn symbol_select(&self, symbol: &str, enable: bool) -> Result<()>` | Select/deselect a symbol in Market Watch (`CMD_SYMBOL_SELECT`). |
| `calendar_events` | `pub fn calendar_events(&self, from: u64, to: u64) -> Result<Vec<CalendarEvent>>` | Economic calendar events in a UTC time window (`CMD_CALENDAR_EVENTS`). |
| `calendar_countries` | `pub fn calendar_countries(&self) -> Result<Vec<String>>` | Countries that have calendar events (`CMD_CALENDAR_COUNTRIES`). |

### Historical Market Data

| Method | Signature | Description |
| :--- | :--- | :--- |
| `copy_rates` | `pub fn copy_rates(&self, symbol: &str, timeframe: Timeframe, from: i64, to: i64) -> Result<Vec<Rate>>` | Raw `Rate` structs within UTC timestamp range `[from, to]`. Rejects requests exceeding 1M bars. |
| `copy_bars` | `pub fn copy_bars(&self, symbol: &str, timeframe: Timeframe, from: i64, to: i64) -> Result<Vec<Bar>>` | Convenience wrapper around `copy_rates` returning clean `Bar` records. |
| `copy_rates_chunked` | `pub fn copy_rates_chunked(&self, symbol: &str, timeframe: Timeframe, start: i64, end: i64, chunk_bars: usize) -> Result<Vec<Rate>>` | Deep history downloader with stabilization retries. |
| `copy_rates_chunked_detailed` | `pub fn copy_rates_chunked_detailed(&self, symbol: &str, timeframe: Timeframe, start: i64, end: i64, chunk_bars: usize) -> Result<HistoryResult>` | Detailed downloader returning `HistoryResult` with completeness tracking. |

### Order & Position Management

| Method | Signature | Description |
| :--- | :--- | :--- |
| `order_send` | `pub fn order_send(&self, req: &OrderRequest) -> Result<TradeResult>` | Submits a market order (Buy/Sell) or pending order (Limit/Stop). |
| `order_close` | `pub fn order_close(&self, ticket: u64) -> Result<TradeResult>` | Closes an open position or cancels a pending order by ticket ID. |
| `close_position` | `pub fn close_position(&self, ticket: u64) -> Result<TradeResult>` | Semantic alias for closing an open market position by ticket ID. |
| `order_close_with_magic` | `pub fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult>` | Closes position with strategy magic-number ownership verification. |
| `close_position_with_magic` | `pub fn close_position_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult>` | Same as `order_close_with_magic`. |
| `order_modify` | `pub fn order_modify(&self, ticket: u64, stop_loss: Level, take_profit: Level) -> Result<TradeResult>` | Modifies SL and TP levels on an existing ticket. |
| `order_modify_with_magic` | `pub fn order_modify_with_magic(&self, ticket: u64, magic: u64, stop_loss: Level, take_profit: Level) -> Result<TradeResult>` | Modifies stops with magic-number ownership verification. |
| `positions` | `pub fn positions(&self) -> Result<Vec<Position>>` | Queries all active open positions in the terminal. |
| `positions_filtered` | `pub fn positions_filtered(&self, magic: Option<u64>, symbol: Option<&str>) -> Result<Vec<Position>>` | Queries open positions matching optional magic and/or symbol filters. |
| `pending_orders` | `pub fn pending_orders(&self) -> Result<Vec<WorkingOrder>>` | Queries all active working pending orders. |
| `pending_orders_filtered` | `pub fn pending_orders_filtered(&self, magic: Option<u64>, symbol: Option<&str>) -> Result<Vec<WorkingOrder>>` | Queries working pending orders with optional filters. |
| `deals` | `pub fn deals(&self, from: i64, to: i64) -> Result<Vec<Deal>>` | Queries completed trade execution history within UTC range `[from, to]`. |
| `deals_filtered` | `pub fn deals_filtered(&self, from: i64, to: i64, magic: Option<u64>, symbol: Option<&str>) -> Result<Vec<Deal>>` | Deal history with optional magic and/or symbol filters. |
| `events_dropped_total` | `pub fn events_dropped_total(&self) -> u64` | Total dropped push-streaming events in the C++ ring buffer. |

---

## Streaming APIs

Real-time streaming built on Tokio async broadcast & mpsc channels (enabled via default `async` feature).

### Real-Time Push Streaming (Protocol v5+)

Event-driven streaming directly fed from MT5's `OnTick()`, `OnBookEvent()`, `OnTradeTransaction()`, and bar closing events:

| Method | Signature | Description |
| :--- | :--- | :--- |
| `subscribe_ticks` | `pub fn subscribe_ticks(&self, symbol: &str) -> Result<TickSubscription>` | Subscribes to push tick events using default `StreamMode::Latest`. |
| `subscribe_ticks_with_mode` | `pub fn subscribe_ticks_with_mode(&self, symbol: &str, mode: StreamMode) -> Result<TickSubscription>` | Subscribe with explicit mode: `Lossless` (backpressure/lag detection) or `Latest` (lowest latency). |
| `unsubscribe_ticks` | `pub fn unsubscribe_ticks(&self, symbol: &str) -> Result<()>` | Unsubscribe from push tick quotes. |
| `subscribe_book` | `pub fn subscribe_book(&self, symbol: &str) -> Result<BookSubscription>` | Subscribe to real-time Level II Market Depth updates. |
| `unsubscribe_book` | `pub fn unsubscribe_book(&self, symbol: &str) -> Result<()>` | Unsubscribe from Level II depth stream. |
| `subscribe_trade` | `pub fn subscribe_trade(&self) -> Result<TradeSubscription>` | Subscribe to real-time broker trade transactions with order comment attribution. |
| `unsubscribe_trade` | `pub fn unsubscribe_trade(&self) -> Result<()>` | Unsubscribe from broker trade transactions stream. |
| `subscribe_bars` | `pub fn subscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<tokio::sync::broadcast::Receiver<Bar>>` | Subscribe to real-time closed-bar notifications. |
| `unsubscribe_bars` | `pub fn unsubscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<()>` | Unsubscribe from closed-bar stream. |

### Polling-Based Streams (Fallback & Candle Aggregation)

| Function | Signature | Description |
| :--- | :--- | :--- |
| `stream_ticks` | `pub fn stream_ticks(client: Arc<Mt5Client>, symbol: &str, poll_interval: Duration) -> mpsc::Receiver<Tick>` | Spawns background task polling `symbol_tick()`, deduplicates identical ticks. |
| `stream_ticks_with_config` | `pub fn stream_ticks_with_config(client, symbol, config: StreamConfig) -> mpsc::Receiver<Tick>` | Tick streaming with explicit buffer size, interval, and backpressure policy. |
| `stream_bars` | `pub fn stream_bars(client, symbol, timeframe, poll_interval, include_incomplete) -> mpsc::Receiver<Result<Bar, StreamError>>` | Emits completed `Bar` on candle close. Extended lookback for calendar intervals (W1, MN1). |

### Stream Backpressure & Loss Semantics

- **`BackpressurePolicy::DropLatest`** (default): When the receiver buffer is full, newly arrived quotes are discarded. Guarantees low latency — consumers always process the latest sampled market state.
- **`BackpressurePolicy::Block`**: When the buffer is full, the background poller awaits channel capacity. Zero message loss — suitable for telemetry, logging, data recording.

---

## Data Structures & Models

### `AccountInfo`

```rust
pub struct AccountInfo {
    pub balance: f64,      // Balance in deposit currency
    pub equity: f64,       // Current equity (balance + floating PnL)
    pub margin: f64,       // Reserved margin
    pub free_margin: f64,  // Available free margin for trading
    pub leverage: f64,     // Account leverage
    pub currency: String,  // Deposit currency (e.g. "USD")
}
```

- `profit(&self) -> f64`: Floating profit/loss (`equity - balance`).
- `margin_level(&self) -> Option<f64>`: Margin percentage (`equity / margin * 100.0`), or `None` if margin is zero.

### `SymbolInfo`

```rust
pub struct SymbolInfo {
    pub symbol: String,    // Symbol name (e.g. "EURUSD")
    pub point: f64,        // Smallest price change unit
    pub tick_value: f64,   // Monetary value of 1 tick per 1.0 lot
    pub tick_size: f64,    // Trade tick size
    pub lot_step: f64,     // Minimum volume increment
    pub min_lot: f64,      // Minimum allowed trade volume
    pub max_lot: f64,      // Maximum allowed trade volume
    pub spread: f64,       // Current spread in points
    pub digits: u32,       // Price decimal places
    pub bid: f64,          // Current bid (Protocol v7)
    pub ask: f64,          // Current ask (Protocol v7)
}
```

- `round_lot(&self, lot: f64) -> f64`: Rounds lot to nearest valid `lot_step`, clamped to `[min_lot, max_lot]`. Returns `0.0` for non-finite, `<= 0`, or sub-minimum lots.
- `is_valid_lot(&self, lot: f64) -> bool`: Verifies min/max/step constraints (rejects NaN/Infinity).
- `point_value(&self, volume: f64) -> f64`: Monetary value of a 1-point move, scaled by `(point / tick_size) * tick_value * volume`.
- `round_price(&self, price: f64) -> f64`: Rounds price to nearest broker `tick_size`, normalized to `digits`.

### `Tick`

```rust
pub struct Tick {
    pub symbol: String,
    pub time_msc: i64,     // milliseconds (UTC)
    pub time: i64,         // seconds (UTC)
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume: u64,
    pub flags: u32,
    pub event_seq: u64,
}
```

- `spread(&self) -> f64`: `ask - bid`.
- `mid(&self) -> f64`: `(ask + bid) / 2.0`.

### `Bar`

```rust
pub struct Bar {
    pub time: i64,         // seconds (UTC)
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}
```

- `mid(&self) -> f64`: `(high + low) / 2.0`
- `typical_price(&self) -> f64`: `(high + low + close) / 3.0`
- `range(&self) -> f64`: `high - low`
- `true_range(&self, prev_close: f64) -> f64`: Max of `high - low`, `\|high - prev_close\|`, `\|low - prev_close\|`.
- `is_bullish(&self) -> bool`: `close > open`.
- `is_bearish(&self) -> bool`: `close < open`.

### `Rate`

Raw 1-to-1 binary equivalent of MT5 `MqlRates`:
```rust
pub struct Rate {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub spread: i32,
    pub real_volume: i64,
}
```

### `HistoryResult`

Returned from `copy_rates_chunked_detailed`:
```rust
pub struct HistoryResult {
    pub rates: Vec<Rate>,
    pub complete: bool,
    pub missing_ranges: Vec<(i64, i64)>,
}
```

### `Timeframe`

Standard chart intervals: M1, M2, M3, M5, M6, M10, M12, M15, M20, M30, H1, H2, H3, H4, H6, H8, H12, D1, W1, MN1.
- `to_mt5_const(self) -> i32`: MT5 `ENUM_TIMEFRAMES` constant.
- `seconds(self) -> i64`: Bar duration in seconds.
- `is_calendar_interval(self) -> bool`: True for W1, MN1.
- `as_str(self) -> &'static str`: Standard code (e.g. `"M15"`).
- `FromStr`: Parses `"m15"`, `"15m"`, `"h1"`, `"1h"`, `"d1"`.

### `OrderRequest`

Fluent builder for trades:
```rust
let req = OrderRequest::buy("EURUSD", 0.1)
    .stop_loss(1.0800)
    .take_profit(1.0950)
    .deviation(10)
    .magic(EA_DEFAULT_MAGIC)
    .comment("my_bot_buy")
    .client_order_id("cid-123");

let req = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 0.1, 1.0820)
    .expiration(1750000000);
```

### `OrderType`

Corresponds to MT5 `ENUM_ORDER_TYPE`: Buy, Sell, BuyLimit, SellLimit, BuyStop, SellStop.
- `is_buy(self) -> bool`: True for Buy, BuyLimit, BuyStop.
- `is_sell(self) -> bool`: True for Sell, SellLimit, SellStop.

### `TradeStatus`

Granular classification: Filled, Placed, PartiallyFilled, Rejected.

### `TradeResult`

Returned from order operations:
```rust
pub struct TradeResult {
    pub retcode: u32,      // MT5 return code (10009 = TRADE_RETCODE_DONE)
    pub deal: u64,         // Deal ticket
    pub order: u64,        // Order ticket
    pub position: u64,     // Position ticket
    pub volume: f64,       // Executed volume
    pub price: f64,        // Execution price
}
```

- `status(&self) -> TradeStatus`: High-level classification.
- `is_success(&self) -> bool`: True if Filled, Placed, or PartiallyFilled.
- `is_filled(&self) -> bool`: True if executed in full as immediate deal.
- `is_placed(&self) -> bool`: True if placed as working pending order.
- `is_partially_filled(&self) -> bool`: True if partially filled.
- `is_deal(&self) -> bool`: True if a deal was executed (`deal > 0`).
- `is_working_order(&self) -> bool`: True if placed without immediate execution.
- `has_position(&self) -> bool`: True if valid non-zero position ticket.
- `description(&self) -> &'static str`: Human-readable explanation of retcode.

### `Position`

```rust
pub struct Position {
    pub ticket: u64,
    pub time: i64,
    pub position_type: OrderType,
    pub magic: u64,
    pub volume: f64,
    pub price_open: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub price_current: f64,
    pub profit: f64,
    pub swap: f64,
    pub symbol: String,
    pub comment: String,
}
```

- `is_buy(&self) -> bool`, `is_sell(&self) -> bool`, `matches_magic(&self, magic: u64) -> bool`.

### `WorkingOrder`

```rust
pub struct WorkingOrder {
    pub ticket: u64,
    pub time_setup: i64,
    pub order_type: OrderType,
    pub magic: u64,
    pub volume_initial: f64,
    pub volume_current: f64,
    pub price_open: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub price_current: f64,
    pub symbol: String,
    pub comment: String,
}
```

### `Deal`

Completed trade execution from MT5 trade history:
```rust
pub struct Deal {
    pub ticket: u64, pub order: u64, pub position_id: u64, pub time: i64,
    pub deal_type: OrderType, pub entry: i32, pub magic: u64,
    pub volume: f64, pub price: f64, pub commission: f64, pub swap: f64,
    pub profit: f64, pub symbol: String, pub comment: String,
}
```

- `is_in(&self) -> bool`: `entry == 0` (position opening/adding).
- `is_out(&self) -> bool`: `entry == 1` (position closing/reducing).

---

## `OrderManager` & Reconciliation Engine

`OrderManager` wraps `Mt5Client` with production-grade execution and recovery guarantees:

- **Order Idempotency**: Maps `client_order_id` to deterministic wire tokens; short-circuits duplicates.
- **Durable Write-Ahead Journaling**: `OrderStore` trait (e.g. `JsonFileStore`) persists intents to disk before wire transmission.
- **Uncertain Execution Handling**: Disconnects after submission map to `OrderState::Unknown`, not blind retries.
- **Broker Reconciliation**: Syncs in-memory orders with live MT5 positions, working orders, deal history.
- **Multi-Pass Grace Period**: `SafetyPolicy` requires multiple absent passes over time (default: 3 passes / 30s) before `Unknown` → `Rejected`.
- **Full Attribute Verification**: Compares symbol, direction, volume, SL, TP; flags discrepancies as `OrderState::Mismatched`.
- **Submission Gates**: `UnknownBlockScope` prevents cascading duplicate exposure.
- **Traceable Retries**: `retry_order()` requires original order to be definitively `Rejected` or `Cancelled`.
- **Multi-Threaded**: `SharedOrderManager` provides atomic locking across check-journal-transmit-record cycle.

See [Trading](trading.md) for usage examples.

---

## Error Handling

All client methods return `Result<T, Mt5Error>`.

### `Mt5Error` Variants

| Variant | Description |
| :--- | :--- |
| `DllLoadError { path, source }` | Failed to load `mt5_bridge.dll`. |
| `InvalidDllPath { path, reason }` | DLL path failed safety validation. |
| `SymbolNotFound { symbol, source }` | Required C ABI symbol missing in DLL. |
| `InitFailed(status)` | Bridge handshake or named pipe connection failed. |
| `CopyRatesFailed { symbol, status }` | Historical rates query failed in MT5. |
| `AccountInfoFailed(status)` | Failed to retrieve balance/equity. |
| `SymbolInfoFailed(symbol)` | Unknown symbol or contract specs unavailable. |
| `SymbolTickFailed(symbol)` | Tick quote query failed. |
| `SymbolsTotalFailed(status)` | Symbols total query failed. |
| `SymbolsGetFailed(status)` | Symbols page query failed. |
| `SymbolSelectFailed(symbol)` | Symbol select/deselect failed. |
| `CalendarEventsFailed(status)` | Calendar events query failed. |
| `CalendarCountriesFailed(status)` | Calendar countries query failed. |
| `StatsFailed(status)` | Bridge statistics query failed. |
| `OrderSendFailed { symbol, retcode, description }` | Order rejected by MT5 terminal/trade server. |
| `OrderCloseFailed { ticket, retcode, description }` | Position closure/cancellation rejected. |
| `OrderModifyFailed { ticket, retcode, description }` | SL/TP modification rejected. |
| `UnknownExecutionState { symbol, client_order_id, description }` | Communication lost after order transmitted; reconciliation required. |
| `TransmissionFailed(description)` | Order transmission to IPC pipe failed prior to broker submission (safe to retry). |
| `OwnershipMismatch { ticket, expected_magic, actual_magic }` | Magic number does not match strategy ownership. |
| `PositionsFailed(status)` | Failed to query active open positions. |
| `OrdersFailed(status)` | Failed to query working pending orders. |
| `ReconciliationError(message)` | State mismatch during broker reconciliation. |
| `AlreadyConnected` | Another client already connected in this process. |
| `UnsupportedFeature(name)` | DLL lacks optional export. |
| `DealsFailed(status)` | Deal history query failed. |
| `ClientOrderIdConflict { client_order_id, reason }` | Client order ID conflicts with an existing tracked order. |
| `UnresolvedUnknownOrders { symbol, client_order_ids }` | Submission blocked by unresolved UNKNOWN orders. |
| `RetryNotAllowed { client_order_id, reason }` | Retry not allowed because the original order is not definitively rejected/cancelled. |
| `PersistenceError(message)` | Order state persistence (write-ahead journal) error. |
| `PreflightRejected(message)` | Order rejected by preflight validation or EA risk gate before transmission. |
| `RiskRejected { rule, detail }` | Order rejected by a pre-trade risk rule. |
| `ServerAuthFailed` | Server authentication or pipe peer verification failed. |
| `PositionsTruncated { returned, total }` | Positions snapshot truncated. |
| `OrdersTruncated { returned, total }` | Working orders snapshot truncated. |
| `DealHistoryTruncated { returned, capacity }` | Deal history snapshot truncated. |
| `InvalidRequest(message)` | Invalid request parameters. |
| `ConfigError(message)` | Configuration error. |
| `PipeDisconnected` | Named pipe disconnected or communication broken. |
| `InvalidTimeRange { start, end }` | Query start timestamp > end timestamp. |
| `ChannelDisconnected` | Async stream receiver/sender disconnected. |
| `Other(message)` | General internal bridge error. |

### Return Code Translator

```rust
use mt5_bridge::mt5_retcode_description;

let msg = mt5_retcode_description(10016);
// "TRADE_RETCODE_INVALID_STOPS: Invalid stops (SL/TP) in the request"
```

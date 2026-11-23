# Code Examples

This page consolidates the usage examples from the README. Each example is self-contained and demonstrates one capability of `mt5-bridge`.

## 1. Connecting to MT5 & Configuration

```rust,no_run
use mt5_bridge::Mt5Client;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect using only the pipe secret token (recommended; keeps broker credentials off IPC):
    let mut client = Mt5Client::connect_with_secret("my_pipe_secret")?;

    // Alternatively, connect specifying account number, password/secret, and server:
    // let mut client = Mt5Client::connect(12345678, "my_pipe_secret", "MetaQuotes-Demo")?;
    println!("Connected to MT5!");

    // Configure in-memory cache TTL for symbol specifications (default: 60s)
    client.set_symbol_cache_ttl(Duration::from_secs(120));

    Ok(())
}
```

> **One client per process.** The bridge keeps its pipe connection and subscription state in process-global DLL variables, so at most one `Mt5Client` may be live per process. A second `connect*` call while a client is alive returns `Mt5Error::AlreadyConnected`. To use several MT5 terminals, run one process per terminal and select its pipe with `MT5_PIPE_NAME`.

*Zero-Config Environment Variables:*
Instead of hardcoding credentials, the client can automatically read:
- `MT5_PIPE_SECRET` or `MT5_PASSWORD` (pipe authentication secret token)
- `MT5_DEMO_ACCOUNT` (set to `1` to confirm demo account for live order placement tests)
- `MT5_LOGIN` (e.g. `12345678` or `0` for active account)
- `MT5_SERVER` (broker server name)
- `MT5_DLL_PATH` (explicit path to `mt5_bridge.dll`)
- `MT5_PIPE_NAME` (custom named pipe name for multi-instance deployments)

*Custom DLL Path:*
```rust,no_run
# use mt5_bridge::Mt5Client;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let client = Mt5Client::connect_with_dll(
    "C:\\path\\to\\mt5_bridge.dll",
    12345678,
    "my_password",
    "MetaQuotes-Demo",
)?;
# Ok(())
# }
```

---

## 2. Fetching Account Information

```rust,no_run
# use mt5_bridge::Mt5Client;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let client = Mt5Client::connect(0, "", "")?;
let account = client.account_info()?;

println!("Balance:      ${:.2}", account.balance);
println!("Equity:       ${:.2}", account.equity);
println!("Margin:       ${:.2}", account.margin);
println!("Free Margin:  ${:.2}", account.free_margin);
println!("Floating PnL: ${:.2}", account.profit());

if let Some(margin_level) = account.margin_level() {
    println!("Margin Level: {:.2}%", margin_level);
} else {
    println!("Margin Level: N/A (no margin currently in use)");
}
# Ok(())
# }
```

---

## 3. Querying Symbol Specifications & Risk Helpers

```rust,no_run
# use mt5_bridge::Mt5Client;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let client = Mt5Client::connect(0, "", "")?;
let info = client.symbol_info("EURUSD")?;

println!("Symbol:     {}", info.symbol);
println!("Point Size: {:.5}", info.point);
println!("Tick Size:  {:.5}", info.tick_size);
println!("Tick Value: {:.2}", info.tick_value);
println!("Min Lot:    {:.2}", info.min_lot);
println!("Max Lot:    {:.2}", info.max_lot);
println!("Lot Step:   {:.2}", info.lot_step);
println!("Digits:     {}", info.digits);

// Helper 1: Round an arbitrary volume to valid broker steps
let valid_lot = info.round_lot(0.1287);
println!("Normalized Lot: {}", valid_lot); // Prints 0.13

// Helper 2: Validate volume satisfies min/max/step constraints (rejects NaN / Inf)
let is_valid = info.is_valid_lot(valid_lot);
println!("Lot is valid: {}", is_valid); // true

// Helper 3: Round arbitrary price to tick size and symbol precision
let valid_price = info.round_price(1.0854321);
println!("Normalized Price: {}", valid_price); // Prints 1.08543

// Helper 4: Accurate multi-asset 1-point move value
let p_val = info.point_value(valid_lot);
println!("1-point move value: ${:.5}", p_val);
# Ok(())
# }
```

---

## 4. Fetching Historical OHLCV Bars & Technical Metrics

```rust,no_run
# use mt5_bridge::Mt5Client;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let client = Mt5Client::connect(0, "", "")?;
use chrono::Utc;
use mt5_bridge::Timeframe;

let now = Utc::now().timestamp();
let one_day_ago = now - 86400;

// Option A: Fetch clean candlestick bars with technical metrics
let bars = client.copy_bars("EURUSD", Timeframe::M15, one_day_ago, now)?;

for bar in bars.iter().take(5) {
    println!(
        "Time: {} | O: {:.5} H: {:.5} L: {:.5} C: {:.5} | Mid: {:.5} | Range: {:.5} | Typical: {:.5} | Bullish: {}",
        bar.time, bar.open, bar.high, bar.low, bar.close,
        bar.mid(), bar.range(), bar.typical_price(), bar.is_bullish()
    );
}

// Option B: Fetch raw 1-to-1 MT5 rates with broker spread and tick/real volumes
let raw_rates = client.copy_rates("EURUSD", Timeframe::M15, one_day_ago, now)?;
if let Some(first) = raw_rates.first() {
    println!("First raw rate spread: {} points | Vol: {}", first.spread, first.volume);
}
# Ok(())
# }
```

---

## 5. Downloading Deep Chunked History with Completeness

```rust,no_run
# use mt5_bridge::Mt5Client;
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let client = Mt5Client::connect(0, "", "")?;
use chrono::Utc;
use mt5_bridge::Timeframe;

let now = Utc::now().timestamp();
let start = now - (30 * 86400); // 30 days ago

// Download deep history with chunking and observable completeness tracking:
let history = client.copy_rates_chunked_detailed("EURUSD", Timeframe::H1, start, now, 200)?;

println!(
    "Downloaded {} bars total (Complete: {}, Missing ranges: {})",
    history.rates.len(),
    history.is_complete(),
    history.missing_ranges.len()
);

if !history.is_complete() {
    for (gap_start, gap_end) in &history.missing_ranges {
        eprintln!("Missing broker history between {} and {}", gap_start, gap_end);
    }
}
# Ok(())
# }
```

---

## 6. Real-Time Tick Streaming (Push Model & Fallback Polling)

`mt5-bridge` (protocol v5+) provides event-driven push tick streaming directly from MT5 `OnTick()`, with dual stream modes:

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

    // Option B: Legacy polling stream with configurable interval
    // let mut rx = mt5_bridge::stream_ticks(Arc::new(client), "EURUSD", Duration::from_millis(10));

    Ok(())
}
```

---

## 7. Real-Time Closed Bar Streaming

Emits clean `Bar` instances whenever a candle closes:

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

---

## 8. Placing, Modifying & Closing Market Orders

```rust,no_run
use mt5_bridge::{Level, Mt5Client, OrderRequest, EA_DEFAULT_MAGIC};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Mt5Client::connect(12345678, "password", "Broker-Demo")?;
    let symbol = "EURUSD";

    let tick = client.symbol_tick(symbol)?;
    let info = client.symbol_info(symbol)?;

    // Calculate tick-aligned SL and TP relative to Bid
    let sl = info.round_price(tick.bid - (200.0 * info.point));
    let tp = info.round_price(tick.bid + (400.0 * info.point));

    // 1. Submit Market Buy order with slippage deviation and custom magic number
    let order_req = OrderRequest::buy(symbol, info.min_lot)
        .stop_loss(sl)
        .take_profit(tp)
        .deviation(15)
        .magic(EA_DEFAULT_MAGIC)
        .comment("bot_trade_1");

    let result = client.order_send(&order_req)?;
    println!(
        "Order opened! Order: {}, Deal: {}, Position: {}, Fill: {:.5}, Status: {:?} ({})",
        result.order, result.deal, result.position, result.price, result.status(), result.description()
    );
    assert!(result.is_filled());

    // In MT5, modifying or closing an executed position uses its position ticket:
    let target_ticket = if result.position > 0 {
        result.position
    } else {
        result.order
    };

    // 2. Modify Stop Loss (move SL closer to market with tick alignment)
    let new_sl = info.round_price(tick.bid - (150.0 * info.point));
    let mod_res = client.order_modify(target_ticket, Level::Set(new_sl), Level::Set(tp))?;
    println!("Stop loss modified! Retcode: {} ({})", mod_res.retcode, mod_res.description());

    // 3. Close the position by position ticket ID using ergonomic close_position API:
    let close_result = client.close_position(target_ticket)?;
    println!(
        "Position closed at {:.5} (Deal: {}, Retcode: {})",
        close_result.price, close_result.deal, close_result.retcode
    );

    // Or close verifying strategy magic number ownership:
    // let close_result = client.close_position_with_magic(target_ticket, EA_DEFAULT_MAGIC)?;

    Ok(())
}
```

---

## 9. Pending Orders with Expiration & Cancellation

```rust,no_run
use chrono::Utc;
use mt5_bridge::{Mt5Client, OrderRequest, OrderType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Mt5Client::connect(12345678, "password", "Broker-Demo")?;
    let symbol = "EURUSD";

    let tick = client.symbol_tick(symbol)?;
    let info = client.symbol_info(symbol)?;

    // Place BuyLimit 200 points below market, expiring in 1 hour
    let limit_price = info.round_price(tick.bid - (200.0 * info.point));
    let expiration = Utc::now().timestamp() + 3600;

    let req = OrderRequest::pending(symbol, OrderType::BuyLimit, info.min_lot, limit_price)
        .expiration(expiration)
        .comment("pending_order_1");

    let trade_res = client.order_send(&req)?;
    println!(
        "BuyLimit placed! Ticket: {}, Status: {:?}, Retcode: {}",
        trade_res.order, trade_res.status(), trade_res.retcode
    );
    assert!(trade_res.is_placed());

    // Cancel pending order by ticket ID
    let cancel_res = client.order_close(trade_res.order)?;
    println!("Pending order cancelled! Retcode: {}", cancel_res.retcode);

    Ok(())
}
```

---

## 10. Error Handling & Return Code Inspection

All bridge operations return typed `Result<T, Mt5Error>`. You can pattern match on errors or inspect MT5 trade retcodes:

```rust,no_run
use mt5_bridge::{mt5_retcode_description, Mt5Client, Mt5Error, OrderRequest};

fn place_trade(client: &Mt5Client, req: &OrderRequest) {
    match client.order_send(req) {
        Ok(trade) => {
            println!("Order executed successfully! Ticket: {}", trade.order);
        }
        Err(Mt5Error::OrderSendFailed { symbol, retcode, description }) => {
            eprintln!("Order rejected for {symbol}: retcode {retcode} ({description})");
        }
        Err(Mt5Error::SymbolInfoFailed(sym)) => {
            eprintln!("Symbol {sym} not found or not selected in Market Watch");
        }
        Err(Mt5Error::Other(msg)) => {
            eprintln!("Bridge error: {msg}");
        }
        Err(e) => {
            eprintln!("Other error: {e}");
        }
    }

    // Direct lookup of any MT5 retcode:
    let msg = mt5_retcode_description(10016);
    println!("Retcode 10016: {msg}");
}
```

---

## 11. Querying Live Positions & Working Orders

Inspect currently active open positions and pending orders directly from MT5:

```rust,no_run
use mt5_bridge::Mt5Client;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Mt5Client::connect(0, "test", "")?;

    // Query all open positions (optionally filtered by magic number or symbol)
    let positions = client.positions_filtered(Some(998877), Some("EURUSD"))?;
    for pos in positions {
        println!(
            "Position #{} on {} | Volume: {:.2} | Open: {:.5} | Profit: ${:.2}",
            pos.ticket, pos.symbol, pos.volume, pos.price_open, pos.profit
        );
    }

    // Query all working pending orders
    let orders = client.pending_orders()?;
    for ord in orders {
        println!(
            "Working Order #{} | Type: {:?} | Volume: {:.2}/{:.2} | Price: {:.5}",
            ord.ticket, ord.order_type, ord.volume_initial, ord.volume_current, ord.price_open
        );
    }

    Ok(())
}
```

---

## 12. Order Idempotency, Lifecycle & Reconciliation Engine

For mission-critical production trading, `OrderManager` wraps `Mt5Client` with:
- **Client Order ID Deduplication**: Replays of existing orders are returned from local memory without sending duplicate orders to the broker.
- **Uncertain Execution Protection**: Disconnects after order submission are flagged as `OrderState::Unknown` rather than triggering dangerous blind retries.
- **Broker State Reconciliation**: Automatically syncs local in-memory orders with live broker state after reconnects or startup.

```rust,no_run
use mt5_bridge::{JsonFileStore, LifecycleState, Mt5Client, OrderManager, OrderRequest, SafetyPolicy};
use std::path::PathBuf;

fn example(client: &Mt5Client) -> Result<(), Box<dyn std::error::Error>> {
    let mut manager = OrderManager::try_new(998877)?; // Strategy Magic 998877

    // Reconcile against live MT5 state on startup
    let report = manager.reconcile(client)?;
    println!(
        "Reconciliation complete (clean: {}). Active positions: {}, working orders: {}",
        report.is_clean(),
        report.positions.len(),
        report.pending_orders.len()
    );

    // Configure write-ahead durable journal
    let store = JsonFileStore::new("orders_journal.json");
    let mut manager = OrderManager::with_store(998877, Box::new(store))?;
    manager.set_policy(SafetyPolicy::default());

    // Reconcile against live MT5 state on startup (evaluates positions, working orders, and deal history)
    let report = manager.reconcile(client)?;
    println!(
        "Reconciliation complete (clean: {}). Active positions: {}, working orders: {}",
        report.is_clean(),
        report.positions.len(),
        report.pending_orders.len()
    );

    // Submit order with deterministic client order ID and write-ahead journaling
    let req = OrderRequest::buy("EURUSD", 0.1)
        .client_order_id("strategy-alpha-001")
        .comment("breakout");

    let tracked = manager.submit_order(client, req)?;
    println!("Order status: {:?}, filled: {:.2}", tracked.state, tracked.filled_volume);

    Ok(())
}
```

#### Durability & Crash Safety (`OrderStore` / `JsonFileStore`)

`OrderManager` supports pluggable durable persistence via the `OrderStore` trait. The built-in `JsonFileStore` implements write-ahead logging:
- Before any packet is transmitted to the broker, the order is recorded in `Submitting` state and atomically flushed/fsynced to disk.
- If the application crashes during network transit, upon restart `restore_orders()` immediately transitions any `Submitting` records to `OrderState::Unknown`, ensuring unconfirmed orders are safely flagged for reconciliation rather than silently forgotten.

#### Multi-Pass Reconciliation Grace Period (`SafetyPolicy`)

To prevent prematurely marking asynchronously visible broker orders as rejected on a single snapshot:
- `SafetyPolicy::default()` requires at least **3 consecutive absent observations** spanning at least **30 seconds** before transitioning an unresolved `Unknown` order to `OrderState::Rejected`.
- Deal history from MT5 (`CMD_DEALS_GET`) is queried automatically during reconciliation to verify whether any executions occurred before an order is declared unplaced.
- Call `reconcile_until_settled` (or `reconcile_until_settled_async` in async runtimes) to poll until all in-flight orders are resolved.

#### Full Attribute Verification & Mismatch Detection

When a live position or working order matches on ticket or wire identity token, `OrderManager` verifies all critical attributes:
- Direction (Buy vs Sell), Symbol, Volume, Stop Loss, and Take Profit.
- Discrepancies are flagged as `OrderState::Mismatched` with specific `AttributeMismatch` categories (e.g. `VolumeMismatch`, `ProtectionMismatch`) rather than silently accepted as clean executions. Operators can acknowledge verified mismatches using `acknowledge_mismatch()`.

#### Submission Gates & Retry Safety (`retry_order`)

- When an order enters `Unknown` state, new submissions for the strategy are automatically blocked under default `UnknownBlockScope::Strategy` (configurable to `Symbol` or `Disabled`) to prevent cascading duplicate exposure.
- Calling `submit_order` with the same `client_order_id` safely short-circuits to the existing tracked order.
- To re-attempt an intent after a definitive failure, callers must use `retry_order`, which strictly enforces that the original order is definitively `Rejected` or `Cancelled` before issuing a traceable retry (`<id>-r1`).

#### Concurrency Contract & Thread Safety (`SharedOrderManager`)

- `OrderManager` is designed with single-ownership semantics (`&mut self`) for maximum single-threaded efficiency.
- For multi-threaded applications, use `SharedOrderManager`:

```rust,no_run
use mt5_bridge::{Mt5Client, OrderManager, OrderRequest, SharedOrderManager};

fn example(client: Mt5Client, req: OrderRequest) -> Result<(), Box<dyn std::error::Error>> {
    let manager = OrderManager::try_new(998877)?;
    let shared = SharedOrderManager::new(manager);

    // Cloneable across threads: holds exclusive lock across idempotency check + write-ahead + broker execution
    let handle = shared.clone();
    std::thread::spawn(move || {
        let _ = handle.submit_order(&client, req);
    });

    Ok(())
}
```

This structurally guarantees that checking idempotency, journaling write-ahead, transmitting over the pipe, and recording the outcome happen atomically across concurrent threads.

**Lock-free reads.** Writes are serialized, but reads are not. After every mutation the manager publishes an `OrderSnapshot` through `arc-swap`, so `get_order`, `export_orders`, `lifecycle`, and `snapshot()` never wait behind an in-flight broker call:

```rust,no_run
# use mt5_bridge::{OrderManager, SharedOrderManager};
# fn example() -> Result<(), Box<dyn std::error::Error>> {
let shared = SharedOrderManager::new(OrderManager::try_new(998877)?);

// Non-blocking, consistent view as of the last completed mutation.
let snapshot = shared.snapshot();
let state = snapshot.lifecycle;

// Authoritative read that never waits: None means "unknown or manager busy".
let latest = shared.try_get_order("ord-1");
# let _ = (snapshot, state, latest);
# Ok(())
# }
```

The snapshot is slightly behind a mutation that is still in flight; use `try_get_order` (or retry) when the authoritative latest state is required.

#### Incremental Event Processing (`apply_trade_event`)

Rather than relying solely on periodic snapshot polling (which introduces latency and pipe overhead), `OrderManager` and `SharedOrderManager` directly consume live push trade notifications via `apply_trade_event`:

```rust,no_run
# use mt5_bridge::{Mt5Client, OrderManager};
# #[cfg(feature = "async")]
# async fn example(client: &Mt5Client, mut manager: OrderManager) -> Result<(), Box<dyn std::error::Error>> {
#     // Subscribe to real-time broker trade events (Protocol v5+)
#     let mut trade_sub = client.subscribe_trade()?;
#     
#     // Background event processing loop; recv_checked() returns Err(StreamError::Lagged) when
#     // the broadcast buffer overflows and events were skipped for this subscriber.
#     tokio::spawn(async move {
#         while let Ok(Some(event)) = trade_sub.recv_checked().await {
#             // Incrementally updates local tracked orders with fills, deal tickets, and closes
#             manager.apply_trade_event(&event);
#         }
#     });
#     Ok(())
# }
```

This enables sub-millisecond local state transitions for fills and closes while reserving full snapshot reconciliation (`manager.reconcile(&client)`) for startup, reconnects, or periodic drift verification.

---

## 13. Integer Tick Pricing Utilities

Eliminate floating-point rounding drift by performing arithmetic in integer price ticks:

```rust,no_run
use mt5_bridge::{calculate_sl_ticks, calculate_tp_ticks, price_to_ticks, ticks_to_price};

let entry_price = 1.08500;
let tick_size = 0.00001;
let digits = 5;

// Convert to integer ticks: 108500
let entry_ticks = price_to_ticks(entry_price, tick_size);

// Calculate exact Stop Loss 50 ticks below entry (1.08450)
let sl_price = calculate_sl_ticks(entry_price, 50, true, tick_size, digits);

// Calculate exact Take Profit 100 ticks above entry (1.08600)
let tp_price = calculate_tp_ticks(entry_price, 100, true, tick_size, digits);
```

---

## Python Examples

Sync client:

```python
import mt5_bridge_py as mb

client = mb.Client.connect_with_secret("test")
acct = client.account_info()
tick = client.symbol_tick("EURUSD")
bars = client.copy_bars("EURUSD", "M5", 1700000000 - 86400, 1700000000)
client.order_send("EURUSD", "buy", 0.01, tick.ask, tick.ask - 0.01, tick.ask + 0.01, "python-test")
client.shutdown()
```

Async client with event streaming:

```python
import asyncio
import mt5_bridge_py as mb

async def main():
    client = mb.AsyncClient.connect_with_secret("test")

    def on_event(event_type: int, payload: bytes):
        print(f"Event type={event_type}, payload_len={len(payload)}")

    client.register_event_callback(on_event)

    count = 0
    async for event in client.events():
        print(f"PyEvent: type={event.event_type}, payload={event.payload[:20]}")
        count += 1
        if count >= 5:
            break

    print(f"Dropped: {client.events_dropped_total()}")
    print(f"Orphans: {client.get_orphan_count()}")

    await client.shutdown()

asyncio.run(main())
```

## Related Documents

- [Python API](python-api.md) — full Python `Client` / `AsyncClient` reference
- [API Reference](api-reference.md) — full Rust `Mt5Client` reference
- [Streaming](streaming.md) — push vs polling streams, backpressure policies
- [Trading](trading.md) — order execution, positions, reconciliation, `OrderManager`

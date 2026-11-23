# Trading & Order Management

## Placing Market Orders

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

    // Submit Market Buy order with slippage deviation and custom magic number
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

    // Modify Stop Loss (move SL closer to market with tick alignment)
    let new_sl = info.round_price(tick.bid - (150.0 * info.point));
    let mod_res = client.order_modify(target_ticket, Level::Set(new_sl), Level::Set(tp))?;
    println!("Stop loss modified! Retcode: {} ({})", mod_res.retcode, mod_res.description());

    // Close the position by position ticket ID
    let close_result = client.close_position(target_ticket)?;
    println!(
        "Position closed at {:.5} (Deal: {}, Retcode: {})",
        close_result.price, close_result.deal, close_result.retcode
    );

    Ok(())
}
```

## Pending Orders with Expiration & Cancellation

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

## Error Handling & Return Code Inspection

All bridge operations return typed `Result<T, Mt5Error>`:

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

## Querying Live Positions & Working Orders

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

## Order Idempotency, Lifecycle & Reconciliation Engine

`OrderManager` wraps `Mt5Client` with production-grade execution and recovery guarantees:

- **Client Order ID Deduplication**: Replays of existing orders are returned from local memory without sending duplicate orders to the broker.
- **Uncertain Execution Protection**: Disconnects after order submission are flagged as `OrderState::Unknown` rather than triggering dangerous blind retries.
- **Broker State Reconciliation**: Automatically syncs local in-memory orders with live broker state after reconnects or startup.
- **Durable Write-Ahead Journaling**: Persists order intents to disk before transmission.

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

    // Reconcile against live MT5 state on startup
    let report = manager.reconcile(client)?;

    // Submit order with deterministic client order ID and write-ahead journaling
    let req = OrderRequest::buy("EURUSD", 0.1)
        .client_order_id("strategy-alpha-001")
        .comment("breakout");

    let tracked = manager.submit_order(client, req)?;
    println!("Order status: {:?}, filled: {:.2}", tracked.state, tracked.filled_volume);

    Ok(())
}
```

### Durability & Crash Safety (`OrderStore` / `JsonFileStore`)

- Before any packet is transmitted to the broker, the order is recorded in `Submitting` state and atomically flushed/fsynced to disk.
- If the application crashes during network transit, upon restart `restore_orders()` immediately transitions any `Submitting` records to `OrderState::Unknown`, ensuring unconfirmed orders are safely flagged for reconciliation rather than silently forgotten.

### Multi-Pass Reconciliation Grace Period (`SafetyPolicy`)

- `SafetyPolicy::default()` requires at least **3 consecutive absent observations** spanning at least **30 seconds** before transitioning an unresolved `Unknown` order to `OrderState::Rejected`.
- Deal history from MT5 (`CMD_DEALS_GET`) is queried automatically during reconciliation to verify whether any executions occurred before an order is declared unplaced.
- Call `reconcile_until_settled` (or `reconcile_until_settled_async` in async runtimes) to poll until all in-flight orders are resolved.

### Full Attribute Verification & Mismatch Detection

When a live position or working order matches on ticket or wire identity token, `OrderManager` verifies all critical attributes:
- Direction (Buy vs Sell), Symbol, Volume, Stop Loss, and Take Profit.
- Discrepancies are flagged as `OrderState::Mismatched` with specific `AttributeMismatch` categories (e.g. `VolumeMismatch`, `ProtectionMismatch`) rather than silently accepted as clean executions. Operators can acknowledge verified mismatches using `acknowledge_mismatch()`.

### Submission Gates & Retry Safety (`retry_order`)

- When an order enters `Unknown` state, new submissions for the strategy are automatically blocked under default `UnknownBlockScope::Strategy` (configurable to `Symbol` or `Disabled`) to prevent cascading duplicate exposure.
- Calling `submit_order` with the same `client_order_id` safely short-circuits to the existing tracked order.
- To re-attempt an intent after a definitive failure, callers must use `retry_order`, which strictly enforces that the original order is definitively `Rejected` or `Cancelled` before issuing a traceable retry (`<id>-r1`).

### Concurrency Contract & Thread Safety (`SharedOrderManager`)

- `OrderManager` is designed with single-ownership semantics (`&mut self`) for maximum single-threaded efficiency.
- For multi-threaded applications, use `SharedOrderManager`:

```rust,no_run
use mt5_bridge::{Mt5Client, OrderManager, OrderRequest, SharedOrderManager};

fn example(client: Mt5Client, req: OrderRequest) -> Result<(), Box<dyn std::error::Error>> {
    let manager = OrderManager::try_new(998877)?;
    let shared = SharedOrderManager::new(manager);

    let handle = shared.clone();
    std::thread::spawn(move || {
        let _ = handle.submit_order(&client, req);
    });

    Ok(())
}
```

- **Lock-free reads.** Writes are serialized, but reads are not. After every mutation the manager publishes an `OrderSnapshot` through `arc-swap`, so `get_order`, `export_orders`, `lifecycle`, and `snapshot()` never wait behind an in-flight broker call.

### Incremental Event Processing (`apply_trade_event`)

`OrderManager` and `SharedOrderManager` directly consume live push trade notifications via `apply_trade_event`:

```rust,no_run
# use mt5_bridge::{Mt5Client, OrderManager};
# #[cfg(feature = "async")]
# async fn example(client: &Mt5Client, mut manager: OrderManager) -> Result<(), Box<dyn std::error::Error>> {
#     let mut trade_sub = client.subscribe_trade()?;
#     tokio::spawn(async move {
#         while let Ok(Some(event)) = trade_sub.recv_checked().await {
#             manager.apply_trade_event(&event);
#         }
#     });
#     Ok(())
# }
```

This enables sub-millisecond local state transitions for fills and closes while reserving full snapshot reconciliation (`manager.reconcile(&client)`) for startup, reconnects, or periodic drift verification.

## Integer Tick Pricing Utilities

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

# Python API Reference

`mt5-bridge-py` provides PyO3 bindings for `mt5-bridge`, exposing a synchronous `Client` and an asynchronous `AsyncClient` (Tokio-backed) to Python. Both require `mt5_bridge.dll` and the `mt5_bridge.mq5` Expert Advisor running inside MetaTrader 5.

Install under Wine Python 3.14:

```bash
# Build the module (maturin) or use the prebuilt .pyd
# Then:
MT5_DLL_REQUIRE_HASH=true wine python.exe -m mt5_bridge_py
```

---

## Quick Start

```python
import mt5_bridge_py as mb

# Connect using the pipe secret token
client = mb.Client.connect_with_secret("test")

# Account information
acct = client.account_info()
print(f"Balance: {acct.balance}, Equity: {acct.equity}")

# Symbol tick
tick = client.symbol_tick("EURUSD")
print(f"Bid: {tick.bid}, Ask: {tick.ask}, Spread: {tick.spread()}")

# Historical bars (M5, last 24h)
import time
now = int(time.time())
bars = client.copy_bars("EURUSD", "M5", now - 86400, now)
print(f"{len(bars)} bars")

client.shutdown()
```

---

## Data Classes

All classes support field get/set via `pyo3(get, set)` and can be constructed from Python:

| Class | Constructor | Key Fields |
| :--- | :--- | :--- |
| `PyAccountInfo` | `PyAccountInfo(balance, equity, margin, free_margin, leverage, currency)` | `balance`, `equity`, `margin`, `free_margin`, `leverage`, `currency` |
| `PySymbolInfo` | `PySymbolInfo()` | `symbol`, `point`, `tick_value`, `tick_size`, `lot_step`, `min_lot`, `max_lot`, `spread`, `digits`, `bid`, `ask` |
| `PyTick` | `PyTick()` | `symbol`, `time_msc`, `time`, `bid`, `ask`, `last`, `volume`, `flags`, `event_seq` |
| `PyBar` | `PyBar(time, open, high, low, close, volume)` | `time`, `open`, `high`, `low`, `close`, `volume` |
| `PyPingResult` | `PyPingResult(server_time_secs, protocol, monotonic_microseconds, rtt_us)` | `server_time_secs`, `protocol`, `monotonic_microseconds`, `rtt_us` |
| `PyBridgeStats` | `PyBridgeStats()` | `uptime_secs`, `requests_total`, `tick_drops`, `trade_drops`, `book_drops`, `bar_drops`, `tick_queue_depth`, `trade_queue_depth`, `last_error`, `protocol`, `latency_p50_us`, `latency_p95_us`, `latency_p99_us` |
| `PyTradeResult` | `PyTradeResult(retcode, deal, order, position, volume, price)` | `retcode`, `deal`, `order`, `position`, `volume`, `price` |
| `PyPosition` | `PyPosition()` | `ticket`, `time`, `position_type`, `magic`, `volume`, `price_open`, `stop_loss`, `take_profit`, `price_current`, `profit`, `swap`, `symbol`, `comment` |
| `PyWorkingOrder` | `PyWorkingOrder()` | `ticket`, `time_setup`, `order_type`, `magic`, `volume_initial`, `volume_current`, `price_open`, `stop_loss`, `take_profit`, `price_current`, `symbol`, `comment` |
| `PyDeal` | `PyDeal()` | `ticket`, `order`, `position_id`, `time`, `direction`, `entry`, `magic`, `volume`, `price`, `commission`, `swap`, `profit`, `symbol`, `comment` |
| `PyCalendarEvent` | `PyCalendarEvent()` | `id`, `time`, `country`, `currency`, `importance`, `title`, `actual`, `forecast`, `previous` |
| `PyEvent` | `PyEvent(event_type, payload)` | `event_type` (u16), `payload` (bytes) |

### Data Class Methods

- `PyAccountInfo.profit()` -> float (equity - balance)
- `PyAccountInfo.margin_level()` -> Optional[float]
- `PyTick.spread()` -> float (ask - bid)
- `PyTick.mid()` -> float ((ask + bid) / 2)
- `PyBar.mid()` -> float
- `PyBar.typical_price()` -> float
- `PyBar.range()` -> float
- `PyBar.is_bullish()` -> bool
- `PyBar.is_bearish()` -> bool
- `PyPingResult.__repr__` -> str
- `PyTradeResult.is_success()` -> bool
- `PyTradeResult.is_filled()` -> bool
- `PyTradeResult.is_placed()` -> bool
- `PyTradeResult.description()` -> str
- `PyCalendarEvent.is_released()` -> bool (True if actual is set)
- `PyEvent.__repr__` -> str

---

## Sync Client

`Client` is the synchronous Python client. One client per process (the DLL stores pipe state in process globals).

### Connection

| Method | Description |
| :--- | :--- |
| `Client()` | Connect with empty secret (convenience) |
| `Client.connect_with_secret(secret: str)` | Class method — connect with pipe secret token |
| `Client.connect(login, password, server)` | Class method — connect with login/password/server |
| `client.shutdown()` | Gracefully disconnect |
| `client.process_connection_count()` | Returns 0 or 1 (process slot occupancy) |

### Market Data

| Method | Returns | Description |
| :--- | :--- | :--- |
| `client.ping()` | `PyPingResult` | Server time, protocol, RTT |
| `client.stats()` | `PyBridgeStats` | Bridge uptime, requests, drops, latency |
| `client.account_info()` | `PyAccountInfo` | Balance, equity, margin |
| `client.symbol_info(symbol)` | `PySymbolInfo` | Contract specs |
| `client.symbol_tick(symbol)` | `PyTick` | Latest quote |
| `client.symbols_total()` | `int` | Total symbols |
| `client.symbols_page(page, page_size)` | `list[PySymbolInfo]` | Paged symbol list |
| `client.copy_bars(symbol, timeframe, from, to)` | `list[PyBar]` | OHLCV bars (M1–D1, etc.) |

### Account & History

| Method | Returns | Description |
| :--- | :--- | :--- |
| `client.positions()` | `list[PyPosition]` | Open positions |
| `client.pending_orders()` | `list[PyWorkingOrder]` | Working pending orders |
| `client.deals(from, to)` | `list[PyDeal]` | Deal history in time range |
| `client.calendar_events(from, to)` | `list[PyCalendarEvent]` | Economic calendar events |
| `client.calendar_countries()` | `list[str]` | Calendar countries |

### Subscriptions (async feature required)

| Method | Returns | Description |
| :--- | :--- | :--- |
| `client.subscribe_ticks(symbol)` | `bool` | Subscribe to push ticks |
| `client.unsubscribe_ticks(symbol)` | `None` | Unsubscribe ticks |
| `client.subscribe_trade()` | `bool` | Subscribe to trade transactions |
| `client.unsubscribe_trade()` | `None` | Unsubscribe trades |
| `client.subscribe_book(symbol)` | `bool` | Subscribe to Level II depth |
| `client.unsubscribe_book(symbol)` | `None` | Unsubscribe book |
| `client.subscribe_bars(symbol, timeframe)` | `bool` | Subscribe to closed-bar push |
| `client.unsubscribe_bars(symbol, timeframe)` | `None` | Unsubscribe bars |

### Symbol Selection

| Method | Description |
| :--- | :--- |
| `client.symbol_select(symbol, enable)` | Select/deselect symbol in Market Watch |

### Trading

| Method | Returns | Description |
| :--- | :--- | :--- |
| `client.order_send(symbol, order_type, volume, price, sl=0.0, tp=0.0, comment="", deviation=None, expiration=None, magic=None)` | `PyTradeResult` | Market or pending order. order_type in {buy, sell, buy_limit, sell_limit, buy_stop, sell_stop} |
| `client.order_close(ticket)` | `PyTradeResult` | Close position / cancel pending order |
| `client.order_close_with_magic(ticket, magic)` | `PyTradeResult` | Close with magic-number ownership check |
| `client.order_modify(ticket, sl_mode, sl, tp_mode, tp)` | `PyTradeResult` | Modify SL/TP (modes: keep, clear, set) |
| `client.order_modify_with_magic(ticket, magic, sl_level, tp_level)` | `PyTradeResult` | Modify with magic check (keep, clear, set:<price>) |

> Bridge safety policy: `order_send` requires `sl > 0` (a stop loss is mandatory). Orders without SL are rejected with "a stop loss is required for every order".

---

## Async AsyncClient (feature async)

`AsyncClient` wraps `Mt5Client` in an Arc with a Tokio runtime, running blocking calls on `spawn_blocking`. All market-data and trading methods are async.

### Connection

| Method | Description |
| :--- | :--- |
| `AsyncClient()` | Connect with empty secret |
| `AsyncClient.connect_with_secret(secret)` | Class method — connect with pipe secret |
| `await client.shutdown()` | Disconnect |

### Async Market Data & Trading

| Method | Returns | Description |
| :--- | :--- | :--- |
| `await client.ping()` | `PyPingResult` | Server time, RTT |
| `await client.stats()` | `PyBridgeStats` | Bridge stats |
| `await client.account_info()` | `PyAccountInfo` | Account info |
| `await client.symbol_info(symbol)` | `PySymbolInfo` | Contract specs |
| `await client.symbol_tick(symbol)` | `PyTick` | Latest quote |
| `await client.copy_bars(symbol, timeframe, from, to)` | `list[PyBar]` | OHLCV bars |
| `await client.order_send(...)` | `PyTradeResult` | Place order (same params as sync) |
| `await client.order_close(ticket)` | `PyTradeResult` | Close position |
| `await client.order_modify(ticket, sl_mode, sl, tp_mode, tp)` | `PyTradeResult` | Modify SL/TP |

### Event System (Task 5)

| Method | Returns | Description |
| :--- | :--- | :--- |
| `client.register_event_callback(cb)` | `int` | Register Python callable (event_type: int, payload: bytes) -> None invoked by the DLL on each streamed event |
| `client.poll_event(timeout_ms)` | `Optional[PyEvent]` | Poll for next event; None on timeout |
| `client.events_dropped_total()` | `int` | DLL ring-buffer dropped-event count |
| `client.get_orphan_count()` | `int` | Orphaned events held by DLL queue |
| `async for event in client.events(): ...` | `PyEvent` | Async event stream (yields PyEvent) |

### Example: Async Event Streaming

```python
import asyncio
import mt5_bridge_py as mb

async def main():
    client = mb.AsyncClient.connect_with_secret("test")

    # Register a callback invoked by the DLL on each event
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

    # Single poll
    ev = client.poll_event(100)
    if ev is not None:
        print(f"Polled: type={ev.event_type}")

    await client.shutdown()

asyncio.run(main())
```

---

## Full Test Suite

A comprehensive Python test suite (101 tests) is committed at:

```
crates/mt5-bridge-py/tests/test_all_features.py
```

It exercises every exposed Python API against a live MT5 terminal under Wine Python 3.14:

| Category | Tests |
| :--- | :--- |
| Data classes | Constructors, methods (spread, mid, profit, margin_level, typical_price, range, is_bullish, is_success, description), __repr__ |
| Sync Client — connection | connect_with_secret, shutdown, re-connect |
| Sync Client — market data | ping, stats, account_info, symbols_total, symbols_page, symbol_info, symbol_tick, copy_bars (M1/M5/H1/D1) |
| Sync Client — account & history | positions, pending_orders, deals, calendar_events, calendar_countries |
| Sync Client — subscriptions | symbol_select, subscribe/unsubscribe ticks/trade/book/bars |
| Sync Client — trading | order_send with valid SL/TP (retcode 10009), order_send without SL (rejected), order_close, order_modify |
| Async Client | ping, stats, account_info, symbol_info, symbol_tick, copy_bars, order_send without SL rejected, shutdown |
| Event System (Task 5) | register_event_callback, poll_event, events_dropped_total, get_orphan_count, async for event in client.events(): |

Run under Wine:

```bash
# Sync tests (one process)
MT5_DLL_REQUIRE_HASH=true wine python.exe crates/mt5-bridge-py/tests/test_all_features.py

# Async tests (separate process — one client per process limit)
MT5_DLL_REQUIRE_HASH=true wine python.exe crates/mt5-bridge-py/tests/test_async_only.py
```

> One client per process: The DLL enforces a single connection per process, so sync and async test suites must run in separate Wine Python processes.

---

## Related Documents

- [Getting Started](getting-started.md) — installation, EA setup, DLL build
- [Benchmarks](benchmarks.md) — Python streaming performance stats
- [Protocol Specification](protocol.md) — binary wire protocol used by the Python bindings

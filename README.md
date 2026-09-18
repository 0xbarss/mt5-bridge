# mt5-bridge

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Author](https://img.shields.io/badge/author-0xbarss-purple.svg)](https://github.com/0xbarss)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Wine-lightgrey.svg)]()
[![MetaTrader 5](https://img.shields.io/badge/MetaTrader-5-green.svg)](https://www.metatrader5.com)

A fast, lightweight, and unofficial native API bridge and client for **MetaTrader 5**, enabling high-performance algorithmic trading and data extraction in **Rust** (and any language supporting C ABI or Named Pipes).

---

## Table of Contents

- [Why mt5-bridge?](#why-mt5-bridge)
- [Architecture](#architecture)
- [Key Features](#key-features)
- [Repository Structure](#repository-structure)
- [Prerequisites](#prerequisites)
- [Step-by-Step Setup Guide](#step-by-step-setup-guide)
  - [Step 1: Install the Expert Advisor in MT5](#step-1-install-the-expert-advisor-in-mt5)
  - [Step 2: Deploy or Build `mt5_bridge.dll`](#step-2-deploy-or-build-mt5_bridgedll)
  - [Step 3: Use the Rust Library](#step-3-use-the-rust-library)
- [Usage & Code Examples](#usage--code-examples)
  - [1. Connecting to MT5](#1-connecting-to-mt5)
  - [2. Fetching Account Information](#2-fetching-account-information)
  - [3. Querying Symbol Specifications](#3-querying-symbol-specifications)
  - [4. Fetching Historical OHLCV Bars](#4-fetching-historical-ohlcv-bars)
  - [5. Real-Time Tick Streaming](#5-real-time-tick-streaming)
  - [6. Real-Time Closed Bar Streaming](#6-real-time-closed-bar-streaming)
  - [7. Placing, Modifying & Closing Orders](#7-placing-modifying--closing-orders)
- [API Reference](#api-reference)
  - [Mt5Client](#mt5client)
  - [Streaming APIs](#streaming-apis)
  - [Data Structures & Models](#data-structures--models)
  - [Error Handling](#error-handling)
- [Multi-Instance / Multi-Account Support](#multi-instance--multi-account-support)
- [Low-Level Binary Protocol](#low-level-binary-protocol)
- [Building the C++ DLL from Source](#building-the-c-dll-from-source)
- [Running on Linux via Wine](#running-on-linux-via-wine)
- [Troubleshooting & FAQ](#troubleshooting--faq)
- [Author & Contributions](#author--contributions)
- [License & Disclaimer](#license--disclaimer)

---

## Why mt5-bridge?

MetaQuotes provides an official Python integration (`MetaTrader5`), but:
- It is **closed-source** and has no public GitHub repository.
- It is **Windows-only** with precompiled C extensions.
- There are **no official libraries for Rust, Go, C++, or other modern systems languages**.
- Many third-party solutions rely on heavy web-sockets, slow REST gateways, or paid commercial bridges.

`mt5-bridge` solves this with an ultra-low-latency, zero-dependency IPC architecture:
1. An **MQL5 Expert Advisor** runs natively inside MetaTrader 5 as a local Named Pipe server.
2. A **C++ Dynamic Library (`mt5_bridge.dll`)** connects to the named pipe and exports clean C ABI functions.
3. A **safe, idiomatic Rust crate** provides typed models, async Tokio streams, retry stabilization loops, and full trade management.

---

## Architecture

```text
┌─────────────────────────────────────────────────────────┐
│                    Rust Application                     │
│                (mt5-bridge client crate)                │
└────────────────────────────┬────────────────────────────┘
                             │
                  Dynamic Linking (libloading)
                             │
┌────────────────────────────▼────────────────────────────┐
│                    mt5_bridge.dll                       │
│             (C++ Named Pipe Client DLL)                 │
└────────────────────────────┬────────────────────────────┘
                             │
           Windows Named Pipe (\\.\pipe\mt5bridge)
                    Binary IPC Protocol
                             │
┌────────────────────────────▼────────────────────────────┐
│                   mt5_bridge.mq5                        │
│          (MQL5 Expert Advisor Pipe Server)              │
└────────────────────────────┬────────────────────────────┘
                             │
                    Internal Terminal API
                             │
┌────────────────────────────▼────────────────────────────┐
│               MetaTrader 5 Client Terminal              │
│                (Broker Trade Server)                    │
└─────────────────────────────────────────────────────────┘
```

- **IPC via Windows Named Pipes**: Operates locally via kernel memory with microsecond latency.
- **Thread-safe**: The C++ DLL serializes requests using Windows `CRITICAL_SECTION`, preventing race conditions.
- **Resilient**: The MQL5 EA non-blockingly polls client connections on a 50ms timer, gracefully handling reconnections.

---

## Key Features

- **Account Overview**: Real-time balance, equity, margin, free margin, and floating profit/loss.
- **Symbol Specifications**: Point size, tick value, contract min/max lots, lot step, spread, and digits with in-memory caching.
- **Historical Data (OHLCV)**: Fetch bars across all standard timeframes (`M1` through `MN1`). Includes chunking and retry loops to allow the broker server to synchronize deep history.
- **Real-Time Streaming**: Asynchronous tick streaming and completed (closed) bar streaming powered by Tokio channels with built-in deduplication.
- **Order Execution**:
  - Instant market orders (`Buy` / `Sell`) with slippage tolerance and order filling flags (`IOC`).
  - Pending orders (`BuyLimit`, `SellLimit`, `BuyStop`, `SellStop`).
  - Position closing by ticket.
  - Stop Loss and Take Profit modification.
- **Multi-Terminal Ready**: Custom named pipe parameters allow running multiple MT5 terminals/accounts concurrently on the same machine.

---

## Repository Structure

```text
mt5-bridge/
├── Cargo.toml               # Rust package manifest
├── LICENSE                  # MIT License
├── README.md                # Documentation (this file)
│
├── mql5/
│   └── Experts/
│       └── mt5_bridge.mq5   # Expert Advisor source code (deploy to MT5)
│
├── bridge_dll/              # C++ Named Pipe client DLL source & builds
│   ├── mt5_bridge.h         # C header and packed struct definitions
│   ├── mt5_bridge.cpp       # Pipe client implementation
│   ├── CMakeLists.txt       # CMake build configuration
│   ├── build.sh             # MinGW cross-compilation script (Linux -> Windows)
│   ├── cmake/
│   │   └── mingw64.cmake    # MinGW toolchain definition
│   └── bin/
│       └── mt5_bridge.dll   # Precompiled 64-bit Windows DLL (ready to use)
│
├── src/                     # Rust library crate
│   ├── lib.rs               # Library entry point & re-exports
│   ├── client.rs            # Safe Mt5Client implementation
│   ├── types.rs             # Typed data structures (Timeframe, Bar, Tick, etc.)
│   ├── error.rs             # Typed error definitions & MT5 retcode translator
│   ├── ffi.rs               # C FFI declarations and packed struct layouts
│   └── stream.rs            # Async Tokio tick & bar stream implementations
│
└── examples/                # Runnable demonstration scripts
    ├── 01_account_info.rs   # Account balance, equity, margin, and margin level
    ├── 02_symbol_info.rs    # Symbol specifications, lot rounding, and point value
    ├── 03_fetch_rates.rs    # Historical OHLCV candles, raw rates, and bar metrics
    ├── 04_stream_ticks.rs   # Real-time live tick quote streaming
    ├── 05_order_send.rs     # Market orders, Stop Loss / Take Profit, and closing
    ├── 06_pending_order.rs  # Pending orders (BuyLimit) and order cancellation
    ├── 07_stream_bars.rs    # Real-time closed-bar streaming via Tokio channels
    └── 08_chunked_history.rs# Deep history downloader with chunking & retry loops
```

---

## Prerequisites

1. **MetaTrader 5 Terminal** (installed on Windows or running via Wine on Linux).
2. **Rust Toolchain**: 1.75 or newer (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).
3. *(Optional)* **C++ Compiler**:
   - If compiling the DLL yourself on Linux: `mingw-w64-gcc` (`x86_64-w64-mingw32-g++`).
   - If compiling on Windows: Visual Studio (MSVC) or MinGW.
   - *Note: A ready-to-use 64-bit DLL is already included in `bridge_dll/bin/mt5_bridge.dll`.*

---

## Step-by-Step Setup Guide

### Step 1: Install the Expert Advisor in MT5

1. Open MetaTrader 5.
2. Click **File** → **Open Data Folder**.
3. Navigate to `MQL5/Experts/` inside the opened explorer window.
4. Copy [`mql5/Experts/mt5_bridge.mq5`](mql5/Experts/mt5_bridge.mq5) into that folder.
5. In MT5, press **F4** to open **MetaEditor** (or double-click `mt5_bridge.mq5`).
6. Press **F7** (or click the **Compile** button). Ensure the compilation finishes with `0 errors, 0 warnings`. This produces `mt5_bridge.ex5`.
7. In the main MT5 terminal, configure settings:
   - Go to **Tools** → **Options** → **Expert Advisors**.
   - Check **"Allow algorithmic trading"**.
   - Check **"Allow DLL imports"** *(essential! The EA uses `kernel32.dll` to create named pipes)*.
   - Go to **Charts** tab: Set **"Max bars in chart"** to **"Unlimited"** (or `1000000`) so deep historical OHLCV queries are not truncated.
   - Click **OK**.
8. Ensure the **"Algo Trading"** button in the top toolbar is **Green** (enabled).
9. From the **Navigator** panel (Ctrl+N), expand **Expert Advisors**, find `mt5_bridge`, and drag it onto **any active chart** (e.g., EURUSD).
10. Check the **Experts** tab at the bottom of MT5. You should see:
    ```text
    MT5Bridge: pipe server ready — waiting for Rust client
    ```

### Step 2: Deploy or Build `mt5_bridge.dll`

The Rust client dynamically loads `mt5_bridge.dll`.

- **Option 1 (Easiest)**: Copy `bridge_dll/bin/mt5_bridge.dll` to your application's working directory, or set the environment variable:
  ```bash
  export MT5_DLL_PATH="/path/to/bridge_dll/bin/mt5_bridge.dll"
  ```
- **Option 2 (Build from source on Linux)**:
  ```bash
  cd bridge_dll
  chmod +x build.sh
  ./build.sh
  ```
- **Option 3 (Build on Windows with MSVC)**:
  ```cmd
  cd bridge_dll
  cl /O2 /LD /DMT5_BRIDGE_EXPORTS mt5_bridge.cpp /link kernel32.lib /OUT:bin\mt5_bridge.dll
  ```

### Step 3: Use the Rust Library

Add `mt5-bridge` to your project's `Cargo.toml`:

```toml
[dependencies]
mt5-bridge = { git = "https://github.com/0xbarss/mt5-bridge" }
tokio = { version = "1.0", features = ["full"] }
```

---

## Usage & Code Examples

### 1. Connecting to MT5

```rust
use mt5_bridge::Mt5Client;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Specify your account number, password, and broker server name.
    // (Note: The MT5 terminal should already be logged in to this account).
    let client = Mt5Client::connect(12345678, "my_password", "MetaQuotes-Demo")?;
    println!("Connected to MT5!");
    Ok(())
}
```

*Custom DLL Path:*
```rust
let client = Mt5Client::connect_with_dll(
    "C:\\path\\to\\mt5_bridge.dll",
    12345678,
    "my_password",
    "MetaQuotes-Demo",
)?;
```

---

### 2. Fetching Account Information

```rust
let account = client.account_info()?;

println!("Balance:      ${:.2}", account.balance);
println!("Equity:       ${:.2}", account.equity);
println!("Margin:       ${:.2}", account.margin);
println!("Free Margin:  ${:.2}", account.free_margin);
println!("Floating PnL: ${:.2}", account.profit());

if let Some(margin_level) = account.margin_level() {
    println!("Margin Level: {:.2}%", margin_level);
}
```

---

### 3. Querying Symbol Specifications

```rust
let info = client.symbol_info("EURUSD")?;

println!("Symbol:     {}", info.symbol);
println!("Point Size: {:.5}", info.point);
println!("Tick Value: {:.2}", info.tick_value);
println!("Min Lot:    {:.2}", info.min_lot);
println!("Max Lot:    {:.2}", info.max_lot);
println!("Lot Step:   {:.2}", info.lot_step);
println!("Digits:     {}", info.digits);

// Helper: Normalize an arbitrary volume to valid broker steps
let valid_lot = info.round_lot(0.1287);
println!("Normalized Lot: {}", valid_lot); // Prints 0.13
```

---

### 4. Fetching Historical OHLCV Bars

```rust
use chrono::Utc;
use mt5_bridge::Timeframe;

let now = Utc::now().timestamp();
let one_day_ago = now - 86400;

// Fetch M15 bars
let bars = client.copy_bars("EURUSD", Timeframe::M15, one_day_ago, now)?;

for bar in bars.iter().take(5) {
    println!(
        "Time: {} | O: {:.5} H: {:.5} L: {:.5} C: {:.5} | Vol: {:.0}",
        bar.time, bar.open, bar.high, bar.low, bar.close, bar.volume
    );
}

// Or use copy_rates_chunked for deep historical data (with stabilization retries):
let deep_history = client.copy_rates_chunked("EURUSD", Timeframe::H1, now - (30 * 86400), now, 2000)?;
println!("Downloaded {} hourly bars", deep_history.len());
```

---

### 5. Real-Time Tick Streaming

Streams real-time price changes via non-blocking Tokio channels:

```rust
use mt5_bridge::stream_ticks;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Mt5Client::connect(12345678, "password", "Broker-Demo")?);

    // Poll every 10 ms for new price ticks
    let mut rx = stream_ticks(client, "EURUSD", Duration::from_millis(10));

    while let Some(tick) = rx.recv().await {
        println!(
            "Tick -> Time: {} | Bid: {:.5} | Ask: {:.5} | Spread: {:.5}",
            tick.time, tick.bid, tick.ask, tick.spread()
        );
    }

    Ok(())
}
```

---

### 6. Real-Time Closed Bar Streaming

Emits clean `Bar` instances whenever a candle closes:

```rust
use mt5_bridge::{stream_bars, Timeframe};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Mt5Client::connect(12345678, "password", "Broker-Demo")?);

    let mut rx = stream_bars(client, "EURUSD", Timeframe::M1, Duration::from_millis(250));

    while let Some(bar) = rx.recv().await {
        println!(
            "Closed Bar -> Time: {} | Open: {:.5} | Close: {:.5} | Range: {:.5}",
            bar.time, bar.open, bar.close, bar.range()
        );
    }

    Ok(())
}
```

---

### 7. Placing, Modifying & Closing Orders

```rust
use mt5_bridge::{Mt5Client, OrderRequest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Mt5Client::connect(12345678, "password", "Broker-Demo")?;

    // 1. Fetch current price
    let tick = client.symbol_tick("EURUSD")?;

    // 2. Open Market Buy order
    let order_req = OrderRequest::buy("EURUSD", 0.01)
        .stop_loss(tick.ask - 0.0050)
        .take_profit(tick.ask + 0.0100)
        .comment("bot_trade_1");

    let result = client.order_send(&order_req)?;
    println!("Order opened! Ticket: {}, Fill Price: {:.5}", result.order, result.price);

    // 3. Modify Stop Loss
    let new_sl = tick.ask - 0.0025;
    client.order_modify(result.order, new_sl, tick.ask + 0.0100)?;
    println!("Stop loss modified!");

    // 4. Close the position
    let close_result = client.order_close(result.order)?;
    println!("Position closed at {:.5} (Deal ticket: {})", close_result.price, close_result.deal);

    Ok(())
}
```

---

## API Reference

Comprehensive reference for public structs, enums, methods, and functions in `mt5-bridge`.

### `Mt5Client`

[`Mt5Client`](src/client.rs) is the primary thread-safe client managing dynamic DLL loading, named pipe IPC, market data queries, and trade execution.

#### Connection & Lifecycle

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`connect`](src/client.rs) | `pub fn connect(login: i64, password: &str, server: &str) -> Result<Self>` | Connects to MT5. Resolves `mt5_bridge.dll` via `MT5_DLL_PATH` environment variable, or checks adjacent to executable / current working directory. |
| [`connect_with_dll`](src/client.rs) | `pub fn connect_with_dll(dll_path: impl AsRef<Path>, login: i64, password: &str, server: &str) -> Result<Self>` | Connects by loading the bridge DLL from an explicit file path. |
| [`set_symbol_cache_ttl`](src/client.rs) | `pub fn set_symbol_cache_ttl(&mut self, ttl: Duration)` | Overrides the in-memory cache time-to-live for `symbol_info` lookups (default: 60 seconds). |
| [`shutdown`](src/client.rs) | `pub fn shutdown(&self) -> Result<()>` | Gracefully disconnects from the MT5 pipe server. Automatically called when the client is dropped (`Drop`). |

#### Account & Symbol Data

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`account_info`](src/client.rs) | `pub fn account_info(&self) -> Result<AccountInfo>` | Queries real-time balance, equity, margin, and free margin in deposit currency. |
| [`symbol_info`](src/client.rs) | `pub fn symbol_info(&self, symbol: &str) -> Result<SymbolInfo>` | Queries contract specifications (point size, lot step, min/max lots, spread, digits) with thread-safe caching. |
| [`symbol_tick`](src/client.rs) | `pub fn symbol_tick(&self, symbol: &str) -> Result<Tick>` | Queries the latest quote tick (bid, ask, last, volume, flags). |

#### Historical Market Data

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`copy_rates`](src/client.rs) | `pub fn copy_rates(&self, symbol: &str, timeframe: Timeframe, from: i64, to: i64) -> Result<Vec<Rate>>` | Fetches raw `Rate` structs within UTC timestamp range `[from, to]`. |
| [`copy_bars`](src/client.rs) | `pub fn copy_bars(&self, symbol: &str, timeframe: Timeframe, from: i64, to: i64) -> Result<Vec<Bar>>` | Convenience wrapper around `copy_rates` that returns clean `Bar` records. |
| [`copy_rates_chunked`](src/client.rs) | `pub fn copy_rates_chunked(&self, symbol: &str, timeframe: Timeframe, start: i64, end: i64, chunk_bars: usize) -> Result<Vec<Rate>>` | Deep history downloader. Fetches history in chunks with stabilization retries to allow MT5 to sync older bars from broker servers. |

#### Order Management

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`order_send`](src/client.rs) | `pub fn order_send(&self, req: &OrderRequest) -> Result<TradeResult>` | Submits a market order (`Buy`/`Sell`) or pending order (`Limit`/`Stop`). |
| [`order_close`](src/client.rs) | `pub fn order_close(&self, ticket: u64) -> Result<TradeResult>` | Closes an open position by ticket ID. |
| [`order_modify`](src/client.rs) | `pub fn order_modify(&self, ticket: u64, stop_loss: f64, take_profit: f64) -> Result<()>` | Modifies Stop Loss and Take Profit levels on an existing ticket. |

---

### Streaming APIs

Asynchronous real-time streaming built on Tokio channels (enabled via default `async` feature).

| Function | Signature | Description |
| :--- | :--- | :--- |
| [`stream_ticks`](src/stream.rs) | `pub fn stream_ticks(client: Arc<Mt5Client>, symbol: &str, poll_interval: Duration) -> mpsc::Receiver<Tick>` | Spawns a background task polling for new ticks, deduplicating unchanged quotes, and emitting new `Tick` values. Task shuts down when receiver is dropped. |
| [`stream_bars`](src/stream.rs) | `pub fn stream_bars(client: Arc<Mt5Client>, symbol: &str, timeframe: Timeframe, poll_interval: Duration) -> mpsc::Receiver<Bar>` | Emits completed (closed) `Bar` structures upon candle close. Skips forming bars and historical initial bars. Task shuts down when receiver is dropped. |

---

### Data Structures & Models

#### `AccountInfo`
[`AccountInfo`](src/types.rs) holds current account equity and margin metrics:
```rust
pub struct AccountInfo {
    pub balance: f64,      // Balance in deposit currency
    pub equity: f64,       // Current equity (balance + floating PnL)
    pub margin: f64,       // Reserved margin
    pub free_margin: f64,  // Available free margin for trading
}
```
- `profit(&self) -> f64`: Returns floating profit/loss (`equity - balance`).
- `margin_level(&self) -> Option<f64>`: Returns margin percentage (`equity / margin * 100.0`), or `None` if margin is zero.

#### `SymbolInfo`
[`SymbolInfo`](src/types.rs) contains contract specifications:
```rust
pub struct SymbolInfo {
    pub symbol: String,    // Symbol name (e.g., "EURUSD")
    pub point: f64,        // Smallest price change unit (e.g. 0.00001)
    pub tick_value: f64,   // Monetary value of 1 tick per 1.0 lot
    pub lot_step: f64,     // Minimum volume increment (e.g. 0.01)
    pub min_lot: f64,      // Minimum allowed trade volume
    pub max_lot: f64,      // Maximum allowed trade volume
    pub spread: f64,       // Current spread in points
    pub digits: u32,       // Price decimal places (e.g. 5)
}
```
- `round_lot(&self, lot: f64) -> f64`: Rounds `lot` to the nearest valid `lot_step`, clamped between `min_lot` and `max_lot`. Returns `0.0` if `lot < min_lot`.
- `is_valid_lot(&self, lot: f64) -> bool`: Verifies whether a lot size satisfies min, max, and step increments.
- `point_value(&self, volume: f64) -> f64`: Returns currency value of a 1-point move for the given volume (`tick_value * volume`).

#### `Tick`
[`Tick`](src/types.rs) represents a live price quote:
```rust
pub struct Tick {
    pub symbol: String,    // Symbol name
    pub time_msc: i64,     // Quote timestamp in milliseconds (UTC)
    pub time: i64,         // Quote timestamp in seconds (UTC)
    pub bid: f64,          // Current bid price
    pub ask: f64,          // Current ask price
    pub last: f64,         // Last deal execution price
    pub volume: u64,       // Volume for last deal
    pub flags: u32,        // MT5 tick flags (TICK_FLAG_BID, etc.)
}
```
- `spread(&self) -> f64`: Returns `ask - bid`.
- `mid(&self) -> f64`: Returns mid-market price `(ask + bid) / 2.0`.

#### `Bar`
[`Bar`](src/types.rs) is a clean OHLCV candle representation:
```rust
pub struct Bar {
    pub time: i64,         // Candle open timestamp in seconds (UTC)
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,       // Tick or real volume
}
```
- `mid(&self) -> f64`: `(high + low) / 2.0`
- `typical_price(&self) -> f64`: `(high + low + close) / 3.0`
- `range(&self) -> f64`: `high - low`
- `true_range(&self, prev_close: f64) -> f64`: Maximum of `high - low`, `|high - prev_close|`, and `|low - prev_close|`.
- `is_bullish(&self) -> bool`: Returns `true` if `close > open`.
- `is_bearish(&self) -> bool`: Returns `true` if `close < open`.

#### `Rate`
[`Rate`](src/types.rs) is the raw 1-to-1 binary equivalent of MT5 `MqlRates`:
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

#### `Timeframe`
[`Timeframe`](src/types.rs) covers standard chart intervals:
- **Variants**: `M1`, `M2`, `M3`, `M5`, `M6`, `M10`, `M12`, `M15`, `M20`, `M30`, `H1`, `H2`, `H3`, `H4`, `H6`, `H8`, `H12`, `D1`, `W1`, `MN1`.
- `to_mt5_const(self) -> i32`: Translates to MT5 `ENUM_TIMEFRAMES` constant.
- `seconds(self) -> i64`: Bar duration in seconds (e.g. `Timeframe::M15.seconds()` -> `900`).
- `as_str(self) -> &'static str`: Returns standard code (e.g. `"M15"`).
- `FromStr`: Parses standard representations (e.g. `"m15"`, `"15m"`, `"h1"`, `"1h"`, `"d1"`).

#### `OrderRequest`
[`OrderRequest`](src/types.rs) provides a fluent builder for trades:
```rust
// Instant Market Orders
let req = OrderRequest::buy("EURUSD", 0.1)
    .stop_loss(1.0800)
    .take_profit(1.0950)
    .comment("my_bot_buy");

let req = OrderRequest::sell("EURUSD", 0.1)
    .stop_loss(1.0950)
    .take_profit(1.0800);

// Pending Orders (Limit / Stop)
let req = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 0.1, 1.0820)
    .stop_loss(1.0770)
    .take_profit(1.0920);
```

#### `OrderType`
[`OrderType`](src/types.rs) corresponds to MT5 `ENUM_ORDER_TYPE`:
- `OrderType::Buy` (0)
- `OrderType::Sell` (1)
- `OrderType::BuyLimit` (2)
- `OrderType::SellLimit` (3)
- `OrderType::BuyStop` (4)
- `OrderType::SellStop` (5)
- `is_buy(self) -> bool`: Returns `true` for `Buy`, `BuyLimit`, `BuyStop`.
- `is_sell(self) -> bool`: Returns `true` for `Sell`, `SellLimit`, `SellStop`.

#### `TradeResult`
[`TradeResult`](src/types.rs) returned from order operations:
```rust
pub struct TradeResult {
    pub retcode: u32,      // MT5 return code (e.g. 10009 = TRADE_RETCODE_DONE)
    pub deal: u64,         // Deal ticket number (if executed)
    pub order: u64,        // Order ticket number
    pub volume: f64,       // Executed trade volume
    pub price: f64,        // Execution price
}
```
- `is_success(&self) -> bool`: Returns `true` if `retcode` is `10009` (Done), `10008` (Placed), or `10010` (Done Partial).
- `description(&self) -> &'static str`: Returns human-readable explanation of `retcode`.

---

### Error Handling

All client methods return [`Result<T, Mt5Error>`](src/error.rs).

#### `Mt5Error`

| Variant | Description |
| :--- | :--- |
| `DllLoadError { path, source }` | Failed to load `mt5_bridge.dll` from specified path. |
| `SymbolNotFound { symbol, source }` | Required C ABI symbol missing in the loaded DLL. |
| `InitFailed(status)` | Bridge handshake or named pipe connection failed. |
| `CopyRatesFailed { symbol, status }` | Historical rates query failed in MT5. |
| `AccountInfoFailed(status)` | Failed to retrieve balance or equity. |
| `SymbolInfoFailed(symbol)` | Unknown symbol or contract specifications unavailable. |
| `SymbolTickFailed(symbol)` | Tick quote query failed. |
| `OrderSendFailed { symbol, retcode, description }` | Order rejected by MT5 terminal / trade server. |
| `OrderCloseFailed { ticket, retcode, description }` | Position closure rejected. |
| `OrderModifyFailed { ticket, retcode }` | SL/TP modification rejected. |
| `UnsupportedFeature(name)` | DLL lacks optional export (e.g. `OrderModify`). |
| `InvalidTimeRange { start, end }` | Query start timestamp is greater than end timestamp. |
| `ChannelDisconnected` | Async stream receiver or sender disconnected. |
| `Other(message)` | General internal bridge error. |

#### Return Code Translator
Call [`mt5_retcode_description(retcode: u32) -> &'static str`](src/error.rs) to inspect any MT5 return code:
```rust
use mt5_bridge::mt5_retcode_description;

let msg = mt5_retcode_description(10016);
println!("{}", msg); // "TRADE_RETCODE_INVALID_STOPS: Invalid stops (SL/TP) in the request"
```

---

## Multi-Instance / Multi-Account Support

To connect to multiple MT5 terminals simultaneously on the same machine:

1. In MT5 Terminal #1:
   - When attaching `mt5_bridge` to the chart, set input parameter `InpPipeName = "mt5bridge_account1"`.
2. In MT5 Terminal #2:
   - Attach `mt5_bridge` with `InpPipeName = "mt5bridge_account2"`.
3. In Rust, direct each client to the corresponding pipe via the `MT5_PIPE_NAME` environment variable:
   ```bash
   MT5_PIPE_NAME=mt5bridge_account1 ./my_bot
   ```

---

## Low-Level Binary Protocol

For developers writing bridges in other languages (Python, Go, C#, Java), the named pipe uses a clean, packed binary protocol over little-endian bytes:

### Request Packet
```text
[uint32 cmd (4 bytes)] [uint32 payload_length (4 bytes)] [payload bytes...]
```

### Response Packet
```text
[int32 status (4 bytes)] [uint32 data_length (4 bytes)] [data bytes...]
```
- `status >= 0`: Success (for `CopyRates`, `status` equals the number of bars returned; for others, `1`).
- `status < 0`: Failure.

### Command Table

| Command ID | Name | Description |
| :---: | :--- | :--- |
| `1` | `CMD_INIT` | Handshake & authentication confirmation |
| `2` | `CMD_SHUTDOWN` | Close named pipe and clean up |
| `3` | `CMD_RATES` | Fetch historical OHLCV bars (`CopyRates`) |
| `4` | `CMD_ACCOUNT` | Query balance, equity, margin, free margin |
| `5` | `CMD_ORDER_SEND` | Send Market or Pending order |
| `6` | `CMD_ORDER_CLOSE` | Close position by ticket |
| `7` | `CMD_ORDER_MODIFY` | Modify SL / TP of an open ticket |
| `8` | `CMD_SYM_TICK` | Query latest tick quote |
| `9` | `CMD_SYM_INFO` | Query symbol contract specifications |

### Packed Struct Layouts (`#pragma pack(push, 1)`)

- **`Mt5SymInfo` (52 bytes)**:
  `double point`, `double tick_value`, `double lot_step`, `double min_lot`, `double max_lot`, `double spread`, `int32 digits`.
- **`Mt5Rate` (60 bytes)**:
  `int64 time`, `double open`, `double high`, `double low`, `double close`, `int64 volume`, `int32 spread`, `int64 real_volume`.
- **`Mt5Tick` (44 bytes)**:
  `int64 time`, `double bid`, `double ask`, `double last`, `uint64 volume`, `uint32 flags`.
- **`Mt5TradeResult` (36 bytes)**:
  `uint32 retcode`, `uint64 deal`, `uint64 order`, `double volume`, `double price`.

---

## Building the C++ DLL from Source

### Cross-Compiling on Linux using MinGW-w64
```bash
# Ubuntu / Debian
sudo apt install gcc-mingw-w64 g++-mingw-w64

# Arch Linux
sudo pacman -S mingw-w64-gcc

# Build
cd bridge_dll
./build.sh
```

### Building on Windows using MSVC
Open `x64 Native Tools Command Prompt for VS`:
```cmd
cd bridge_dll
cl /O2 /LD /DMT5_BRIDGE_EXPORTS mt5_bridge.cpp /link kernel32.lib /OUT:bin\mt5_bridge.dll
```

---

## Running on Linux via Wine

MetaTrader 5 runs smoothly under Wine on Linux.
When using `mt5-bridge`:
1. Ensure MT5 is installed and running inside your Wine prefix (`wine terminal64.exe`).
2. Attach `mt5_bridge.mq5` to a chart inside MT5.
3. Wine implements Windows Named Pipes through Unix domain sockets or standard Wine IPC (`\\.\pipe\mt5bridge` translates to `~/.wine/drive_c/...` or socket mapping).
4. Run your Rust trading engine compiled for Windows (`x86_64-pc-windows-gnu` via Wine) or use a socket bridge if running a purely native Linux process.

---

## Troubleshooting & FAQ

#### 1. "CreateNamedPipe failed — check DLL imports are enabled"
- **Cause**: MT5 blocked `kernel32.dll` access.
- **Fix**: Open MT5 → **Tools** → **Options** → **Expert Advisors** → check **"Allow DLL imports"**. Recompile and reattach the EA.

#### 2. "OrderSend failed (retcode 10027)"
- **Cause**: Algorithmic trading is disabled.
- **Fix**: Click the **"Algo Trading"** button in the main MT5 toolbar (it should turn green), and ensure "Allow algorithmic trading" is checked in the EA inputs.

#### 3. "Failed to load MT5 bridge DLL"
- **Cause**: `mt5_bridge.dll` was not found in the search path or current working directory.
- **Fix**: Set the `MT5_DLL_PATH` environment variable:
  ```bash
  export MT5_DLL_PATH="/absolute/path/to/mt5_bridge.dll"
  ```

#### 4. "Retcode 10014: Invalid volume in the request"
- **Cause**: The lot volume requested does not conform to the broker's minimum or lot step size.
- **Fix**: Use `info.round_lot(requested_volume)` to automatically normalize volume.

#### 5. "Retcode 10016: Invalid stops (SL/TP)"
- **Cause**: Stop loss or take profit is placed too close to the current price (within broker freeze levels or stops level).
- **Fix**: Check `SymbolInfoDouble(sym, SYMBOL_TRADE_STOPS_LEVEL)` and place stops outside that distance.

#### 6. "Historical data is truncated or `copy_rates` returns fewer bars than requested"
- **Cause**: MetaTrader 5 limits the maximum number of bars saved and cached per chart by default (often 100,000 or fewer).
- **Fix**: Open MT5 → **Tools** → **Options** → **Charts** tab. Set **"Max bars in chart"** to **"Unlimited"** (or at least `1000000` / `1_000_000`), click **OK**, and restart the MT5 terminal.

---
 
## Author & Contributions
 
Created and maintained by [**0xbarss**](https://github.com/0xbarss).
 
Contributions, bug reports, and feature suggestions are welcome! Please check out [**CONTRIBUTING.md**](CONTRIBUTING.md) for development guidelines, testing instructions, and commit conventions before submitting pull requests. Feel free to open an issue or pull request at [**github.com/0xbarss/mt5-bridge**](https://github.com/0xbarss/mt5-bridge).
 
---
 
## License & Disclaimer

This project is licensed under the **[MIT License](LICENSE)**.

> **Disclaimer**: This is an **unofficial** open-source project and is not affiliated, associated, authorized, endorsed by, or in any way officially connected with MetaQuotes Ltd. or MetaTrader 5. Trading foreign exchange, CFDs, and cryptocurrencies carries a high level of risk. Always test your code on a Demo account before running live strategies.

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
- [Architecture & Design](#architecture--design)
  - [IPC Architecture](#ipc-architecture)
  - [Security Model & Threat Assumptions](#security-model--threat-assumptions)
  - [Trade Ownership & Magic Number Scope](#trade-ownership--magic-number-scope)
  - [Concurrency, Latency & Serialization](#concurrency-latency--serialization)
  - [Timezone & Historical Timestamps Contract](#timezone--historical-timestamps-contract)
- [Key Features](#key-features)
- [Repository Structure](#repository-structure)
- [Prerequisites & Installation](#prerequisites--installation)
  - [Step 1: Install the Expert Advisor in MT5](#step-1-install-the-expert-advisor-in-mt5)
  - [Step 2: Deploy or Build `mt5_bridge.dll`](#step-2-deploy-or-build-mt5_bridgedll)
  - [Step 3: Running on Linux via Wine](#step-3-running-on-linux-via-wine)
  - [Step 4: Add Rust Crate to Project](#step-4-add-rust-crate-to-project)
- [Usage & Code Examples](#usage--code-examples)
  - [1. Connecting to MT5 & Configuration](#1-connecting-to-mt5--configuration)
  - [2. Fetching Account Information](#2-fetching-account-information)
  - [3. Querying Symbol Specifications & Risk Helpers](#3-querying-symbol-specifications--risk-helpers)
  - [4. Fetching Historical OHLCV Bars & Technical Metrics](#4-fetching-historical-ohlcv-bars--technical-metrics)
  - [5. Downloading Deep Chunked History with Completeness](#5-downloading-deep-chunked-history-with-completeness)
  - [6. Real-Time Tick Streaming](#6-real-time-tick-streaming)
  - [7. Real-Time Closed Bar Streaming](#7-real-time-closed-bar-streaming)
  - [8. Placing, Modifying & Closing Market Orders](#8-placing-modifying--closing-market-orders)
  - [9. Pending Orders with Expiration & Cancellation](#9-pending-orders-with-expiration--cancellation)
  - [10. Error Handling & Return Code Inspection](#10-error-handling--return-code-inspection)
- [API Reference](#api-reference)
  - [Mt5Client](#mt5client)
  - [Streaming APIs](#streaming-apis)
  - [Data Structures & Models](#data-structures--models)
  - [Error Handling](#error-handling)
- [Multi-Instance / Multi-Account Support](#multi-instance--multi-account-support)
- [Low-Level Binary IPC Protocol](#low-level-binary-ipc-protocol)
  - [Packet Framing](#packet-framing)
  - [Command Table](#command-table)
  - [Packed Struct Layouts](#packed-struct-layouts)
- [Building the C++ DLL from Source](#building-the-c-dll-from-source)
- [Testing & Quality Assurance](#testing--quality-assurance)
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

## Architecture & Design

### IPC Architecture

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
- **Thread-safe & Resilient I/O**: The C++ DLL serializes requests using Windows `CRITICAL_SECTION`, preventing packet interleaving. When pipe I/O breaks or the terminal closes, the DLL automatically tears down and invalidates the pipe handle (`disconnect_locked`) to fail fast and cleanly reconnect.
- **Bounded High-Throughput EA Timer Loop**: The MQL5 EA non-blockingly polls client connections on an optimized 5ms timer loop (`InpTimerIntervalMs`), draining queued requests with strict execution bounds (`InpMaxRequestsPerTimer = 32` and `InpMaxTimerBudgetUs = 2000 µs`). This prevents request floods from ever starving MT5's single-threaded event loop or freezing the terminal UI.

### Security Model & Threat Assumptions

The bridge operates across an Inter-Process Communication (IPC) boundary between the client engine and the MetaTrader 5 Expert Advisor:

1. **Local IPC Boundary**: The bridge uses Windows Named Pipes (`\\.\pipe\...`). All communication is strictly local to the machine running the MT5 terminal.
2. **Persistent Authentication State**: The EA enforces connection authentication state. All incoming commands (`CMD_ORDER_SEND`, `CMD_ACCOUNT`, `CMD_RATES`, etc.) are rejected with an error unless preceded by a valid, authenticated `CMD_INIT` handshake.
3. **Wire Protocol Versioning**: `CMD_INIT` negotiates wire protocol versioning (`PROTOCOL_VERSION = 2`). Version mismatches between the client DLL and the EA are rejected immediately, guaranteeing ABI compatibility for packed structs.
4. **Shared Secret Token**: `InpPipeSecret` provides application-level authentication. For production deployments, configure a non-empty secret on the EA and provide it via the `MT5_PIPE_SECRET` environment variable. Set `InpRequireSecret = true` to prevent the EA from starting without a secret configured.
5. **Terminal Account Verification**: `HandleInit` verifies that the requested account login and trade server match the active MT5 terminal connection (`ACCOUNT_LOGIN` and `ACCOUNT_SERVER`), preventing accidental execution against the wrong account.
6. **Memory & Bounds Safety**: All packet parsing helpers enforce strict bounds checks before reading (`SafeUnpack*`), rejecting truncated or malformed payloads without crashing the EA event loop.
7. **Pre-Flight Validation**: Both the Rust client and MQL5 EA enforce pre-flight validation. The EA verifies that the symbol is enabled for trading (`SYMBOL_TRADE_MODE_DISABLED`), checks pending order limits, validates lot sizes against broker min/max/step constraints (`SYMBOL_VOLUME_STEP`), checks stops and freeze level distances (`SYMBOL_TRADE_STOPS_LEVEL`), ensures tick size alignment (`SYMBOL_TRADE_TICK_SIZE`), and verifies prices, Stop Loss, and Take Profit values before submission.
8. **Deterministic Position Resolution (Hedging Safe)**: In hedging accounts with multiple positions per symbol, the bridge strictly resolves position IDs via deal history (`DEAL_POSITION_ID`) or ticket selection (`PositionSelectByTicket`). It deliberately avoids ambiguous symbol-only lookups (`PositionSelect(sym)`); if position tracking cannot be verified, `position = 0` is safely returned instead of guessing an arbitrary position ticket.
9. **Market Watch Control**: `InpAutoSelectSymbols` (default `true`) allows configuring whether queries automatically select symbols into Market Watch or strictly require them to already exist.
10. **Non-Blocking Pipe Peeking**: The EA inspects available pipe buffer lengths before calling read operations, ensuring a stalled or crashed client cannot freeze the MetaTrader 5 UI or chart timer thread.

### Trade Ownership & Magic Number Scope

- **Bridge Magic Number**: The EA attaches `InpMagicNumber` (default `20240101`) to all orders placed through the bridge.
- **Strict vs Account-Wide Management**:
  - By default (`InpEnforceMagicNumber = false`), `order_close` and `order_modify` allow managing any position or pending order on the account, logging a warning if the ticket was opened manually or by another EA.
  - When `InpEnforceMagicNumber = true`, operations on tickets whose magic number does not match `InpMagicNumber` are strictly rejected. Manual trades (magic `0`) and unassigned orders are also strictly disallowed with no zero-bypass.
- **Custom Order Magic**: Callers can override the magic number per-request using `OrderRequest::buy(...).magic(my_magic)`.

### Concurrency, Latency & Serialization

- **Single-Channel Serialization**: All requests through `Mt5Client` are serialized through a Win32 `CRITICAL_SECTION` in `mt5_bridge.dll` and handled sequentially by the MQL5 EA on a timer loop.
- **Bounded Timer Draining**: The EA runs an optimized 5ms timer (`InpTimerIntervalMs = 5`) and drains buffered pipe requests up to `InpMaxRequestsPerTimer` (32) or `InpMaxTimerBudgetUs` (2000 µs) per tick. This yields ultra-low latency while preserving MT5 terminal UI responsiveness.
- **Broken Pipe Invalidation**: On pipe I/O failure (`ERROR_BROKEN_PIPE`, broken socket/pipe, or client crash), the C++ DLL automatically closes and invalidates the pipe handle (`disconnect_locked()`), allowing downstream callers to handle the error immediately without deadlocking.
- **Multi-Symbol Throughput Guidelines**: For multi-symbol streaming, configure balanced polling intervals (e.g. 50ms–100ms across 10+ symbols) to prevent pipe queue serialization backpressure, or run separate dedicated MT5 terminal instances with independent pipe names (`InpPipeName`).
- **Latency Expectations**:
  - Live order execution (`order_send`, `order_close`) takes typical local pipe turn-around plus broker execution round-trip latency.
  - Large historical data requests (`copy_rates` or `copy_rates_chunked`) can take seconds as MT5 queries the broker history server.
  - For high-frequency trading where bulk history downloads must not delay trade execution, run separate dedicated MT5 terminal instances with independent pipe names (`InpPipeName`).

### Timezone & Historical Timestamps Contract

- **UTC Conversion**: By default (`InpConvertToUTC = true`), the EA normalizes rates and tick timestamps to UTC unix timestamps using the current broker server offset (`TimeTradeServer() - TimeGMT()`).
- **Daylight Saving Time (DST)**: Forex brokers frequently shift offsets between UTC+2 (winter) and UTC+3 (summer). If exact historical candle alignment across DST transitions is critical, disable conversion (`InpConvertToUTC = false`) to receive native broker trade server timestamps.
- **Calendar Timeframes (`MN1`, `W1`)**: Monthly (`MN1`) and weekly (`W1`) bars have variable durations. `copy_rates_chunked` provides `copy_rates_chunked_detailed` with completeness metadata and missing range tracking to ensure data integrity during backtesting. Additionally, `stream_bars` automatically expands lookback windows for calendar intervals (6 weeks for `W1`, 6 months for `MN1`) to guarantee completed candle delivery.

---

## Key Features

- **Account Overview**: Real-time balance, equity, margin, free margin, and floating profit/loss.
- **Symbol Specifications**: Point size, tick size, tick value, contract min/max lots, lot step, spread, and digits with in-memory caching.
- **Price & Lot Normalization**: Helpers to round prices to tick size and normalize lots to valid broker steps, with `is_valid_lot()` bounds checking against `NaN` and `Infinity`.
- **Historical Data (OHLCV)**: Fetch bars across all standard timeframes (`M1` through `MN1`). Includes chunking and retry loops to allow the broker server to synchronize deep history, plus `copy_rates_chunked_detailed()` reporting completeness.
- **Real-Time Streaming**: Asynchronous tick streaming and completed (closed) bar streaming powered by Tokio channels with built-in deduplication and automatic cancellation.
- **Order Execution**:
  - Instant market orders (`Buy` / `Sell`) with slippage tolerance and order filling flags.
  - Pending orders (`BuyLimit`, `SellLimit`, `BuyStop`, `SellStop`) with expiration control.
  - Position closing by ticket.
  - Stop Loss and Take Profit modification with tick-aligned precision and retcode feedback.
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
│       ├── mt5_bridge.mq5   # Expert Advisor source code (deploy to MT5)
│       └── mt5_bridge.ex5   # Compiled Expert Advisor binary
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
├── tests/                   # Test suites
│   ├── bridge_tests.rs      # Unit & data model tests (offline, CI-ready)
│   └── live_integration.rs  # End-to-end integration tests (requires running MT5)
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

## Prerequisites & Installation

1. **MetaTrader 5 Terminal** (installed on Windows or running via Wine on Linux).
2. **Rust Toolchain**: 1.75 or newer (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).
3. *(Optional)* **C++ Compiler**:
   - If compiling the DLL yourself on Linux: `mingw-w64-gcc` (`x86_64-w64-mingw32-g++`).
   - If compiling on Windows: Visual Studio (MSVC) or MinGW.
   - *Note: A ready-to-use 64-bit DLL is already included in `bridge_dll/bin/mt5_bridge.dll`.*

### Step 1: Install the Expert Advisor in MT5

1. Open MetaTrader 5.
2. Click **File** → **Open Data Folder**.
3. Navigate to `MQL5/Experts/` inside the opened explorer window.
4. Copy [`mql5/Experts/mt5_bridge.mq5`](mql5/Experts/mt5_bridge.mq5) (and optionally [`mt5_bridge.ex5`](mql5/Experts/mt5_bridge.ex5)) into that folder.
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
    MT5Bridge: pipe server ready (timer=5ms) — waiting for Rust client
    ```

#### Expert Advisor Input Parameters

| Parameter | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `InpMagicNumber` | `int` | `20240101` | Magic number assigned to bridge trades |
| `InpPipeName` | `string` | `"mt5bridge"` | Named pipe name (override for multi-terminal setups) |
| `InpPipeSecret` | `string` | `""` | Optional shared secret token for client authentication |
| `InpRequireSecret` | `bool` | `false` | Require non-empty secret token before allowing initialization |
| `InpEnforceMagicNumber` | `bool` | `false` | Strictly reject modify/close for tickets not matching `InpMagicNumber` (no zero-bypass) |
| `InpConvertToUTC` | `bool` | `true` | Convert broker history and tick timestamps to UTC (disable for raw broker time) |
| `InpTimerIntervalMs` | `int` | `5` | Timer polling and pipe draining loop frequency in milliseconds |
| `InpMaxRequestsPerTimer` | `int` | `32` | Max requests serviced per timer tick (prevents terminal UI starvation) |
| `InpMaxTimerBudgetUs` | `uint` | `2000` | Max execution budget per timer tick in microseconds (2000 µs = 2 ms) |
| `InpAutoSelectSymbols` | `bool` | `true` | Automatically select queried/traded symbols into Market Watch |

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

### Step 3: Running on Linux via Wine

MetaTrader 5 runs smoothly under Wine on Linux:
1. Ensure MT5 is installed and running inside your Wine prefix (`wine terminal64.exe`).
2. Attach `mt5_bridge.mq5` to a chart inside MT5.
3. Wine implements Windows Named Pipes through Unix domain sockets or standard Wine IPC (`\\.\pipe\mt5bridge`).
4. To run Rust examples/applications against Wine MT5:
   ```bash
   # Add the Windows target to Rust
   rustup target add x86_64-pc-windows-gnu

   # Build and run with Wine
   cargo build --target x86_64-pc-windows-gnu --example 01_account_info
   wine target/x86_64-pc-windows-gnu/debug/examples/01_account_info.exe
   ```

### Step 4: Add Rust Crate to Project

Add `mt5-bridge` to your project's `Cargo.toml`:

```toml
[dependencies]
mt5-bridge = { git = "https://github.com/0xbarss/mt5-bridge" }
tokio = { version = "1.0", features = ["full"] }
```

---

## Usage & Code Examples

### 1. Connecting to MT5 & Configuration

```rust
use mt5_bridge::Mt5Client;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Connect specifying account number, password (or pipe secret), and server.
    // (Pass 0 and empty strings to attach to the terminal's active logged-in account).
    let mut client = Mt5Client::connect(12345678, "my_password", "MetaQuotes-Demo")?;
    println!("Connected to MT5!");

    // 2. Configure in-memory cache TTL for symbol specifications (default: 60s)
    client.set_symbol_cache_ttl(Duration::from_secs(120));

    Ok(())
}
```

*Zero-Config Environment Variables:*
Instead of hardcoding credentials, the client can automatically read:
- `MT5_LOGIN` (e.g. `12345678` or `0` for active account)
- `MT5_PASSWORD` or `MT5_PIPE_SECRET` (pipe authentication secret)
- `MT5_SERVER` (broker server name)
- `MT5_DLL_PATH` (explicit path to `mt5_bridge.dll`)
- `MT5_PIPE_NAME` (custom named pipe name for multi-instance deployments)

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
} else {
    println!("Margin Level: N/A (no margin currently in use)");
}
```

---

### 3. Querying Symbol Specifications & Risk Helpers

```rust
let info = client.symbol_info("EURUSD")?;

println!("Symbol:     {}", info.symbol);
println!("Point Size: {:.5}", info.point);
println!("Tick Size:  {:.5}", info.tick_size);
println!("Tick Value: {:.2}", info.tick_value);
println!("Min Lot:    {:.2}", info.min_lot);
println!("Max Lot:    {:.2}", info.max_lot);
println!("Lot Step:   {:.2}", info.lot_step);
println!("Digits:     {}", info.digits);

// Helper 1: Round an arbitrary volume to valid broker steps (returns 0.0 for sub-minimum or invalid/NaN/negative lots)
let valid_lot = info.round_lot(0.1287);
println!("Normalized Lot: {}", valid_lot); // Prints 0.13

// Helper 2: Validate volume satisfies min/max/step constraints (rejects NaN / Inf)
let is_valid = info.is_valid_lot(valid_lot);
println!("Lot is valid: {}", is_valid); // true

// Helper 3: Round arbitrary price to tick size and symbol precision
let valid_price = info.round_price(1.0854321);
println!("Normalized Price: {}", valid_price); // Prints 1.08543

// Helper 4: Accurate multi-asset 1-point move value (scaled by point / tick_size)
let p_val = info.point_value(valid_lot);
println!("1-point move value: ${:.5}", p_val);
```

---

### 4. Fetching Historical OHLCV Bars & Technical Metrics

```rust
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
```

---

### 5. Downloading Deep Chunked History with Completeness

```rust
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
```

---

### 6. Real-Time Tick Streaming

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

    let mut count = 0;
    while let Some(tick) = rx.recv().await {
        println!(
            "Tick -> Time: {} | Bid: {:.5} | Ask: {:.5} | Spread: {:.5} | Last: {:.5}",
            tick.time, tick.bid, tick.ask, tick.spread(), tick.last
        );
        count += 1;
        if count >= 10 {
            // Dropping receiver automatically terminates the background polling task
            break;
        }
    }

    Ok(())
}
```

---

### 7. Real-Time Closed Bar Streaming

Emits clean `Bar` instances whenever a candle closes:

```rust
use mt5_bridge::{stream_bars, Timeframe};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Mt5Client::connect(12345678, "password", "Broker-Demo")?);

    // Listen for newly closed 1-minute bars
    let mut rx = stream_bars(client, "EURUSD", Timeframe::M1, Duration::from_millis(500));

    while let Some(bar) = rx.recv().await {
        println!(
            "Closed Candle -> Time: {} | Open: {:.5} | Close: {:.5} | Range: {:.5} | Vol: {:.0}",
            bar.time, bar.open, bar.close, bar.range(), bar.volume
        );
    }

    Ok(())
}
```

---

### 8. Placing, Modifying & Closing Market Orders

```rust
use mt5_bridge::{Mt5Client, OrderRequest};

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
        .magic(20240101)
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
    let mod_res = client.order_modify(target_ticket, new_sl, tp)?;
    println!("Stop loss modified! Retcode: {} ({})", mod_res.retcode, mod_res.description());

    // 3. Close the position by position ticket ID
    let close_result = client.order_close(target_ticket)?;
    println!(
        "Position closed at {:.5} (Deal: {}, Retcode: {})",
        close_result.price, close_result.deal, close_result.retcode
    );

    Ok(())
}
```

---

### 9. Pending Orders with Expiration & Cancellation

```rust
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

### 10. Error Handling & Return Code Inspection

All bridge operations return typed `Result<T, Mt5Error>`. You can pattern match on errors or inspect MT5 trade retcodes:

```rust
use mt5_bridge::{mt5_retcode_description, Mt5Client, Mt5Error, OrderRequest};

fn place_trade(client: &Mt5Client, req: &OrderRequest) {
    match client.order_send(req) {
        Ok(trade) => {
            println!("Order executed successfully! Ticket: {}", trade.order);
        }
        Err(Mt5Error::OrderSendFailed { symbol, retcode, description }) => {
            eprintln!("Order rejected for {symbol}: retcode {retcode} ({description})");
        }
        Err(Mt5Error::RatesLimitExceeded { requested, max }) => {
            eprintln!("Request exceeded limit: {requested} bars > {max} maximum");
        }
        Err(Mt5Error::SymbolInfoFailed(sym)) => {
            eprintln!("Symbol {sym} not found or not selected in Market Watch");
        }
        Err(e) => {
            eprintln!("Bridge error: {e}");
        }
    }

    // Direct lookup of any MT5 retcode:
    let msg = mt5_retcode_description(10016);
    println!("Retcode 10016: {msg}");
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
| [`symbol_info`](src/client.rs) | `pub fn symbol_info(&self, symbol: &str) -> Result<SymbolInfo>` | Queries contract specifications (point size, tick size, tick value, lot step, min/max lots, spread, digits) with thread-safe caching. |
| [`symbol_tick`](src/client.rs) | `pub fn symbol_tick(&self, symbol: &str) -> Result<Tick>` | Queries the latest quote tick (bid, ask, last, volume, flags). |

#### Historical Market Data

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`copy_rates`](src/client.rs) | `pub fn copy_rates(&self, symbol: &str, timeframe: Timeframe, from: i64, to: i64) -> Result<Vec<Rate>>` | Fetches raw `Rate` structs within UTC timestamp range `[from, to]`. Rejects requests exceeding 1M bars with `RatesLimitExceeded`. |
| [`copy_bars`](src/client.rs) | `pub fn copy_bars(&self, symbol: &str, timeframe: Timeframe, from: i64, to: i64) -> Result<Vec<Bar>>` | Convenience wrapper around `copy_rates` that returns clean `Bar` records. |
| [`copy_rates_chunked`](src/client.rs) | `pub fn copy_rates_chunked(&self, symbol: &str, timeframe: Timeframe, start: i64, end: i64, chunk_bars: usize) -> Result<Vec<Rate>>` | Deep history downloader. Fetches history in chunks with stabilization retries. |
| [`copy_rates_chunked_detailed`](src/client.rs) | `pub fn copy_rates_chunked_detailed(&self, symbol: &str, timeframe: Timeframe, start: i64, end: i64, chunk_bars: usize) -> Result<HistoryResult>` | Detailed downloader returning `HistoryResult` with completeness tracking. |

#### Order Management

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`order_send`](src/client.rs) | `pub fn order_send(&self, req: &OrderRequest) -> Result<TradeResult>` | Submits a market order (`Buy`/`Sell`) or pending order (`Limit`/`Stop`). |
| [`order_close`](src/client.rs) | `pub fn order_close(&self, ticket: u64) -> Result<TradeResult>` | Closes an open position or cancels a pending order by ticket ID. |
| [`order_modify`](src/client.rs) | `pub fn order_modify(&self, ticket: u64, stop_loss: f64, take_profit: f64) -> Result<TradeResult>` | Modifies Stop Loss and Take Profit levels on an existing ticket. |

---

### Streaming APIs

Asynchronous real-time streaming built on Tokio channels (enabled via default `async` feature).

| Function | Signature | Description |
| :--- | :--- | :--- |
| [`stream_ticks`](src/stream.rs) | `pub fn stream_ticks(client: Arc<Mt5Client>, symbol: &str, poll_interval: Duration) -> mpsc::Receiver<Tick>` | Spawns a background task that polls for new quotes via `symbol_tick()`, deduplicates identical ticks (inspecting time, bid, ask, last, volume, and flags), and yields updated `Tick` values. Operates via latest-quote polling (not a lossless queue). Task terminates when receiver is dropped. |
| [`stream_bars`](src/stream.rs) | `pub fn stream_bars(client: Arc<Mt5Client>, symbol: &str, timeframe: Timeframe, poll_interval: Duration) -> mpsc::Receiver<Bar>` | Emits completed (closed) `Bar` structures upon candle close. Skips forming bars and historical initial bars. Automatically applies extended lookback windows for calendar intervals (`W1`, `MN1`). Task shuts down when receiver is dropped. |

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
    pub tick_size: f64,    // Trade tick size (e.g. 0.00001 or 0.25)
    pub lot_step: f64,     // Minimum volume increment (e.g. 0.01)
    pub min_lot: f64,      // Minimum allowed trade volume
    pub max_lot: f64,      // Maximum allowed trade volume
    pub spread: f64,       // Current spread in points
    pub digits: u32,       // Price decimal places (e.g. 5)
}
```
- `round_lot(&self, lot: f64) -> f64`: Rounds `lot` to the nearest valid `lot_step`, clamped between `min_lot` and `max_lot`. Returns `0.0` if `lot` is non-finite (`NaN`, `Infinity`), `<= 0.0`, or strictly below `min_lot` (prevents silent risk inflation).
- `is_valid_lot(&self, lot: f64) -> bool`: Verifies whether a lot size satisfies min, max, and step increments (rejects `NaN` and `Infinity`).
- `point_value(&self, volume: f64) -> f64`: Calculates the true monetary value of a 1-point price move for the given volume, properly scaled by `(point / tick_size) * tick_value * volume` for CFDs, indices, and forex.
- `round_price(&self, price: f64) -> f64`: Rounds `price` to the nearest broker `tick_size` and normalizes to symbol `digits`.

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

#### `HistoryResult`
[`HistoryResult`](src/types.rs) returned from `copy_rates_chunked_detailed`:
```rust
pub struct HistoryResult {
    pub rates: Vec<Rate>,                // Retrieved historical rates
    pub complete: bool,                  // True if all requested ranges were retrieved
    pub missing_ranges: Vec<(i64, i64)>, // Ranges where broker returned no data
}
```
- `is_complete(&self) -> bool`: Returns `true` if all requested ranges were retrieved without error.

#### `Timeframe`
[`Timeframe`](src/types.rs) covers standard chart intervals:
- **Variants**: `M1`, `M2`, `M3`, `M5`, `M6`, `M10`, `M12`, `M15`, `M20`, `M30`, `H1`, `H2`, `H3`, `H4`, `H6`, `H8`, `H12`, `D1`, `W1`, `MN1`.
- `to_mt5_const(self) -> i32`: Translates to MT5 `ENUM_TIMEFRAMES` constant.
- `seconds(self) -> i64`: Bar duration in seconds (e.g. `Timeframe::M15.seconds()` -> `900`).
- `is_calendar_interval(self) -> bool`: Returns `true` for variable-length calendar periods (`W1`, `MN1`).
- `as_str(self) -> &'static str`: Returns standard code (e.g. `"M15"`).
- `FromStr`: Parses standard representations (e.g. `"m15"`, `"15m"`, `"h1"`, `"1h"`, `"d1"`).

#### `OrderRequest`
[`OrderRequest`](src/types.rs) provides a fluent builder for trades:
```rust
// Instant Market Orders with custom deviation & magic
let req = OrderRequest::buy("EURUSD", 0.1)
    .stop_loss(1.0800)
    .take_profit(1.0950)
    .deviation(10)
    .magic(20240101)
    .comment("my_bot_buy");

let req = OrderRequest::sell("EURUSD", 0.1)
    .stop_loss(1.0950)
    .take_profit(1.0800);

// Pending Orders (Limit / Stop) with expiration
let req = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 0.1, 1.0820)
    .stop_loss(1.0770)
    .take_profit(1.0920)
    .expiration(1750000000);
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

#### `TradeStatus`
[`TradeStatus`](src/types.rs) provides granular classification:
- `TradeStatus::Filled`: Market order filled in full.
- `TradeStatus::Placed`: Pending order placed and active in terminal.
- `TradeStatus::PartiallyFilled`: Order partially executed.
- `TradeStatus::Rejected`: Order rejected or failed.

#### `TradeResult`
[`TradeResult`](src/types.rs) returned from order operations:
```rust
pub struct TradeResult {
    pub retcode: u32,      // MT5 return code (e.g. 10009 = TRADE_RETCODE_DONE)
    pub deal: u64,         // Deal ticket number (if executed)
    pub order: u64,        // Order ticket number
    pub position: u64,     // Position ticket number associated with the trade
    pub volume: f64,       // Executed trade volume
    pub price: f64,        // Execution price
}
```
- `retcode: u32`: **Authoritative MT5 return code** (e.g. `10009` for `TRADE_RETCODE_DONE`). All downstream decision logic should inspect this field.
- `position: u64`: Position ticket number (0 if pending or unknown). In hedging accounts, position tickets are safely identified via `DEAL_POSITION_ID` or ticket selection without guessing.
- `status(&self) -> TradeStatus`: High-level convenience classification (`Filled` for immediate deal executions, `Placed` for working pending orders).
- `is_success(&self) -> bool`: Returns `true` if `status` is `Filled`, `Placed`, or `PartiallyFilled`.
- `is_filled(&self) -> bool`: Returns `true` if executed in full as an immediate deal.
- `is_placed(&self) -> bool`: Returns `true` if placed as a pending order and currently working.
- `is_partially_filled(&self) -> bool`: Returns `true` if partially filled.
- `is_deal(&self) -> bool`: Returns `true` if a deal was executed (`deal > 0`).
- `is_working_order(&self) -> bool`: Returns `true` if an order was placed without immediate execution (`deal == 0 && order > 0`).
- `has_position(&self) -> bool`: Returns `true` if a valid non-zero position ticket is assigned.
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
| `RatesLimitExceeded { requested, max }` | Requested historical range exceeds maximum single-request limit (1,000,000 bars). |
| `AccountInfoFailed(status)` | Failed to retrieve balance or equity. |
| `SymbolInfoFailed(symbol)` | Unknown symbol or contract specifications unavailable. |
| `SymbolTickFailed(symbol)` | Tick quote query failed. |
| `OrderSendFailed { symbol, retcode, description }` | Order rejected by MT5 terminal / trade server. |
| `OrderCloseFailed { ticket, retcode, description }` | Position closure or pending order cancellation rejected. |
| `OrderModifyFailed { ticket, retcode, description }` | SL/TP modification rejected with MT5 retcode. |
| `UnsupportedFeature(name)` | DLL lacks optional export. |
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

## Low-Level Binary IPC Protocol

For developers writing bridges in other languages (Python, Go, C#, Java), the named pipe uses a clean, packed binary protocol over little-endian bytes:

### Packet Framing

#### Request Packet
```text
[uint32 cmd (4 bytes)] [uint32 payload_length (4 bytes)] [payload bytes...]
```

#### Response Packet
```text
[int32 status (4 bytes)] [uint32 data_length (4 bytes)] [data bytes...]
```
- `status >= 0`: Success (for `CopyRates`, `status` equals the number of bars returned; for others, `1`).
- `status < 0`: Failure.

### Command Table

| Command ID | Name | Description |
| :---: | :--- | :--- |
| `1` | `CMD_INIT` | Handshake, authentication confirmation & protocol version (`PROTOCOL_VERSION = 2`) |
| `2` | `CMD_SHUTDOWN` | Close named pipe and clean up |
| `3` | `CMD_RATES` | Fetch historical OHLCV bars (`CopyRates`) clamped by buffer capacity |
| `4` | `CMD_ACCOUNT` | Query balance, equity, margin, free margin |
| `5` | `CMD_ORDER_SEND` | Send Market or Pending order |
| `6` | `CMD_ORDER_CLOSE` | Close position or cancel pending order by ticket |
| `7` | `CMD_ORDER_MODIFY` | Modify SL / TP of an open ticket |
| `8` | `CMD_SYM_TICK` | Query latest tick quote |
| `9` | `CMD_SYM_INFO` | Query symbol contract specifications |

### Packed Struct Layouts (`#pragma pack(push, 1)`)

- **`Mt5SymInfo` (60 bytes)**:
  `double point`, `double tick_value`, `double tick_size`, `double lot_step`, `double min_lot`, `double max_lot`, `double spread`, `int32 digits`.
- **`Mt5Rate` (60 bytes)**:
  `int64 time`, `double open`, `double high`, `double low`, `double close`, `int64 volume`, `int32 spread`, `int64 real_volume`.
- **`Mt5Tick` (44 bytes)**:
  `int64 time`, `double bid`, `double ask`, `double last`, `uint64 volume`, `uint32 flags`.
- **`Mt5TradeResult` (44 bytes)**:
  `uint32 retcode`, `uint64 deal`, `uint64 order`, `uint64 position`, `double volume`, `double price`.

### ABI Consistency & Wire Protocol Versioning

The bridge enforces strict compile-time and runtime alignment across the C++ DLL, MQL5 EA, and Rust FFI:
- **Wire Protocol Version**: Handshake version `PROTOCOL_VERSION = 2` (defined as `MT5_BRIDGE_PROTOCOL_VERSION` in C++ and `PROTOCOL_VERSION` in MQL5 and Rust).
- **Compile-Time ABI Assertions**: Struct byte layouts are validated via C++11 `static_assert` and Rust compile-time layout assertions:
  - `Mt5SymInfo`: 60 bytes
  - `Mt5Rate`: 60 bytes
  - `Mt5Tick`: 44 bytes
  - `Mt5TradeResult`: 44 bytes
- **Handshake Verification**: `CMD_INIT` passes the client's protocol version. If there is a version mismatch between the client DLL and the EA server, the connection is rejected immediately to prevent binary deserialization faults.

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

## Testing & Quality Assurance

The codebase includes two dedicated test suites under [`tests/`](tests):

### 1. Offline Unit & Property Tests
Comprehensive unit tests covering timeframe math, calendar intervals, lot rounding edge cases, valid lot checks with `NaN`/`Infinity` guards, price rounding, point value scaling, order builders, and trade status classifications:
```bash
cargo test --test bridge_tests
```

### 2. Live Integration Suite
Tests executed directly against an active MetaTrader 5 terminal:
```bash
# On Windows or via Wine:
cargo test --target x86_64-pc-windows-gnu --test live_integration
```
*Note: The live test suite utilizes a thread-safe mutex and an RAII `OrderGuard` pattern to ensure that even in the case of test panics, all placed pending and market orders are automatically cancelled or closed in `Drop`. The suite also incorporates market-closure safety (handling retcode `10018`) and streaming timeouts to allow safe execution during weekends or market closures.*

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
- **Fix**: Check `SymbolInfoDouble(sym, SYMBOL_TRADE_STOPS_LEVEL)` and place stops outside that distance using `info.round_price()`.

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

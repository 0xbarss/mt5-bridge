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
  - [Trade Execution State Machine](#trade-execution-state-machine)
  - [Event Processing Model](#event-processing-model)
  - [Queue Overflow & Observability](#queue-overflow--observability)
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
  - [6. Real-Time Tick Streaming (Push Model & Fallback Polling)](#6-real-time-tick-streaming-push-model--fallback-polling)
  - [7. Real-Time Closed Bar Streaming](#7-real-time-closed-bar-streaming)
  - [8. Placing, Modifying & Closing Market Orders](#8-placing-modifying--closing-market-orders)
  - [9. Pending Orders with Expiration & Cancellation](#9-pending-orders-with-expiration--cancellation)
  - [10. Error Handling & Return Code Inspection](#10-error-handling--return-code-inspection)
  - [11. Querying Live Positions & Working Orders](#11-querying-live-positions--working-orders)
  - [12. Order Idempotency, Lifecycle & Reconciliation Engine](#12-order-idempotency-lifecycle--reconciliation-engine)
  - [13. Integer Tick Pricing Utilities](#13-integer-tick-pricing-utilities)
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
  - [1. Offline Unit & Property Tests](#1-offline-unit--property-tests)
  - [2. Live Integration Suite (Active MT5 Terminal)](#2-live-integration-suite-active-mt5-terminal)
  - [3. Failure Injection Matrix](#3-failure-injection-matrix)
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
│                     mt5_bridge.dll                      │
│              (C++ Named Pipe Client DLL)                │
└────────────────────────────┬────────────────────────────┘
                             │
          Windows Named Pipe (\\.\pipe\mt5bridge)
                    Binary IPC Protocol
                             │
┌────────────────────────────▼────────────────────────────┐
│                    mt5_bridge.mq5                       │
│          (MQL5 Expert Advisor Pipe Server)              │
└────────────────────────────┬────────────────────────────┘
                             │
                  Internal Terminal API
                             │
┌────────────────────────────▼────────────────────────────┐
│               MetaTrader 5 Client Terminal              │
│                  (Broker Trade Server)                  │
└─────────────────────────────────────────────────────────┘
```

- **IPC via Windows Named Pipes**: Operates locally via kernel memory with microsecond latency.
- **Thread-safe & Resilient I/O**: The C++ DLL serializes requests using Windows `CRITICAL_SECTION`, preventing packet interleaving. When pipe I/O breaks or the terminal closes, the DLL automatically tears down and invalidates the pipe handle (`disconnect_locked`) to fail fast and cleanly reconnect.
- **Bounded High-Throughput EA Timer Loop**: The MQL5 EA non-blockingly polls client connections on an optimized 5ms timer loop (`InpTimerIntervalMs`), draining queued requests with strict execution bounds (`InpMaxRequestsPerTimer = 32` and `InpMaxTimerBudgetUs = 2000 µs`). This prevents request floods from ever starving MT5's single-threaded event loop or freezing the terminal UI.

### Security Model & Threat Assumptions

The bridge operates across an Inter-Process Communication (IPC) boundary between the client engine and the MetaTrader 5 Expert Advisor:

1. **Local IPC Boundary**: The bridge uses Windows Named Pipes (`\\.\pipe\...`). All communication is strictly local to the machine running the MT5 terminal.
2. **Persistent Authentication State**: The EA enforces connection authentication state. All incoming commands (`CMD_ORDER_SEND`, `CMD_ACCOUNT`, `CMD_RATES`, etc.) are rejected with an error unless preceded by a valid, authenticated `CMD_INIT` handshake.
3. **Wire Protocol Versioning**: `CMD_INIT` negotiates wire protocol versioning (`PROTOCOL_VERSION = 5`). Version mismatches between the client DLL and the EA are rejected immediately, guaranteeing ABI compatibility for packed structs.
4. **Shared Secret Token**: `InpPipeSecret` provides application-level authentication. `InpRequireSecret` is enabled by default (`true`), preventing the EA from starting without a secret configured (set `InpRequireSecret = false` to opt out). Provide the secret from Rust via [`Mt5Client::connect_with_secret`](#) or the `MT5_PIPE_SECRET` environment variable.
5. **Explicit Pipe Security Descriptor (ACL)**: The named pipe is created with an explicit Win32 Security Descriptor (`InpPipeSDDL = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"`), restricting pipe access strictly to the owner user account, Local System, and Administrators, preventing unauthorized local users or cross-session processes from connecting.
6. **Terminal Account Verification**: `HandleInit` verifies that the requested account login and trade server match the active MT5 terminal connection (`ACCOUNT_LOGIN` and `ACCOUNT_SERVER`), preventing accidental execution against the wrong account.
7. **Memory & Bounds Safety**: All packet parsing helpers enforce strict bounds checks before reading (`SafeUnpack*`), rejecting truncated or malformed payloads without crashing the EA event loop.
8. **Pre-Flight Validation**: Both the Rust client and MQL5 EA enforce pre-flight validation. The EA verifies that the symbol is enabled for trading (`SYMBOL_TRADE_MODE_DISABLED`), checks pending order limits, validates lot sizes against broker min/max/step constraints (`SYMBOL_VOLUME_STEP`), checks stops and freeze level distances (`SYMBOL_TRADE_STOPS_LEVEL`), ensures tick size alignment (`SYMBOL_TRADE_TICK_SIZE`), and verifies prices, Stop Loss, and Take Profit values before submission.
9. **Deterministic Position Resolution (Hedging Safe)**: In hedging accounts with multiple positions per symbol, the bridge strictly resolves position IDs via deal history (`DEAL_POSITION_ID`) or ticket selection (`PositionSelectByTicket`). It deliberately avoids ambiguous symbol-only lookups (`PositionSelect(sym)`); if position tracking cannot be verified, `position = 0` is safely returned instead of guessing an arbitrary position ticket.
10. **Market Watch Control**: `InpAutoSelectSymbols` (default `true`) allows configuring whether queries automatically select symbols into Market Watch or strictly require them to already exist.
11. **Non-Blocking Pipe Peeking**: The EA inspects available pipe buffer lengths before calling read operations, ensuring a stalled or crashed client cannot freeze the MetaTrader 5 UI or chart timer thread.

### Trade Execution State Machine

A primary vulnerability in automated trading bridges is treating a transport failure (e.g., named pipe disconnection, socket timeout, or process restart) as an order rejection. If a client blindly retries an order that was actually executed by the broker, catastrophic duplicate exposure occurs.

To prevent this, `mt5-bridge` formally models execution via a deterministic state machine:

```text
                     ┌───────────────┐
                     │    Created    │
                     └───────┬───────┘
                             │
                             ▼
                     ┌───────────────┐
                     │  Submitting   │
                     └───────┬───────┘
                             │
           ┌─────────────────┼─────────────────┐
           │                 │                 │
           ▼                 ▼                 ▼
    ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
    │  Accepted   │   │   Unknown   │   │  Rejected   │
    └──────┬──────┘   └──────┬──────┘   └─────────────┘
           │                 │
           │                 ▼
           │          ┌─────────────┐
           │          │  Reconcile  │
           │          └──────┬──────┘
           │                 │
           │       ┌─────────┴─────────┐
           │       │                   │
           │       ▼                   ▼
           │ ┌───────────┐     ┌───────────────┐
           │ │ Confirmed │     │ Still Unknown │
           │ └─────┬─────┘     └───────┬───────┘
           │       │                   │ (grace expired)
           │       │                   ▼
           │       │           ┌───────────────┐
           │       │           │   Rejected    │
           │       │           └───────────────┘
           │       │
           ▼       ▼
      ┌───────────────────┐
      │ Partially Filled  │
      └─────────┬─────────┘
                │
                ▼
      ┌───────────────────┐
      │      Filled       │
      └─────────┬─────────┘
                │
                ▼
      ┌───────────────────┐
      │      Closed       │
      └───────────────────┘
```

> [!IMPORTANT]
> **Core Invariant: `Transport failure ≠ order rejection`**
> An order whose confirmation was lost over the wire is never assumed dead. It enters `OrderState::Unknown`, blocks further submissions under `UnknownBlockScope::Strategy`, and initiates multi-pass verification against MT5 live positions and deal history via `manager.reconcile(&client)`. Only when definitive absence is confirmed across consecutive observations spanning the grace period (default: 3 passes and 30 seconds) is the order marked `Rejected`.

### Event Processing Model

A production-grade algorithmic trading architecture strictly separates **transport**, **events**, **in-memory state**, and **broker reconciliation**:

```text
                 ┌───────────────────┐
                 │ Initial Snapshot  │
                 └─────────┬─────────┘
                           │
                           ▼
                 ┌───────────────────┐
                 │    Local State    │
                 └─────────▲─────────┘
                           │
              ┌────────────┴────────────┐
              │                         │
              ▼                         ▼
    ┌───────────────────┐     ┌───────────────────┐
    │ Trade/Book Events │     │ Periodic Snapshot │
    └─────────┬─────────┘     └─────────┬─────────┘
              │                         │
              ▼                         ▼
    ┌───────────────────┐     ┌───────────────────┐
    │    Apply Event    │     │     Reconcile     │
    └─────────┬─────────┘     └─────────┬─────────┘
              │                         │
              └────────────┬────────────┘
                           │
                           ▼
                 ┌───────────────────┐
                 │ Consistent State  │
                 └───────────────────┘
```

#### Event-Snapshot Lifecycle
1. **Initial Snapshot**: Upon startup or reconnect, [`OrderManager::reconcile(&client)`](src/reconciliation.rs) queries active positions (`CMD_POSITIONS_GET`), pending orders (`CMD_ORDERS_GET`), and historical deals (`CMD_DEALS_GET`), establishing an exact baseline of current broker exposure.
2. **Incremental Push Stream**: The MQL5 EA monitors [`OnTradeTransaction()`](mql5/Experts/mt5_bridge.mq5) and pushes structured [`Mt5TradeEvent`](src/ffi.rs) payloads into the named pipe. The EA automatically propagates trade comments (from order requests, deal history, or order history), enabling [`OrderManager::apply_trade_event(&event)`](src/reconciliation.rs) to instantly attribute fills, partial executions, and closes to tracked client order IDs without polling overhead.
3. **Periodic Reconcile Pass**: A lightweight background snapshot reconciles all tracked open positions against MT5 state, detecting any discrepancies (e.g. manual broker intervention, slippage, off-bridge closes) and resolving any in-flight orders.
4. **Consistent State Guarantee**: If event delivery lags or drops under severe load, the periodic reconciliation engine corrects any drift, guaranteeing eventual consistency without risking out-of-order corruption.

### Queue Overflow & Observability

In high-volatility market conditions (e.g. major news releases), quote or transaction generation can outpace consumer processing. `mt5-bridge` ensures pipeline backpressure and event loss are strictly observable:

- **C++ Ring Buffer with Overwrite Protection**: The C++ DLL maintains an internal circular ring buffer (default 65,536 slots). When downstream consumers fail to drain events before the buffer cycles, older unread events are overwritten safely without memory leaks, and an atomic counter `g_events_dropped_total` is incremented.
- **Client Drop Counter Inspection**: Call [`Mt5Client::events_dropped_total(&self) -> u64`](src/client.rs) at any time to monitor dropped events across all active subscriptions.
- **MQL5 EA Drops Alerting**: The EA maintains independent queue counters (`g_ticks_dropped_total`, `g_trades_dropped_total`) and emits periodic warning alerts to the MT5 Experts log whenever queue capacity thresholds are exceeded.
- **Dual Stream Modes**:
  - `StreamMode::Lossless`: Backpressure detection via `recv_checked()` which returns `Err(RecvError::Lagged(skipped))` whenever a consumer falls behind, guaranteeing that recording and auditing engines detect data gaps.
  - `StreamMode::Latest`: Broadcast channel that drops lagged quotes in favor of the newest tick, ensuring execution algorithms never execute on a stale backlog.

### Trade Ownership & Magic Number Scope

- **Bridge Magic Number**: The EA attaches `InpMagicNumber` (default `20240101`) to all orders placed through the bridge unless a custom magic number is specified.
- **Multi-Strategy & Magic Management**:
  - By default (`InpEnforceMagicNumber = false`), orders placed with custom magic numbers (or multiple strategies sharing the same bridge connection) can be freely closed and modified by ticket. When closing or modifying positions, the bridge preserves the position's original magic number in the trade record.
  - To restrict the bridge strictly to a single magic number, set `InpEnforceMagicNumber = true`. When enabled, the EA strictly enforces `InpMagicNumber` across `OrderSend`, `OrderClose`, and `OrderModify`, rejecting any operation where the magic number does not match.
- **Custom Order Magic**: Callers can specify custom magic numbers per-request using `OrderRequest::buy(...).magic(my_magic)`.

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
├── bridge_dll/              # C++ Named Pipe client DLL source & build scripts
│   ├── mt5_bridge.h         # C header and packed struct definitions
│   ├── mt5_bridge.cpp       # Pipe client implementation
│   ├── CMakeLists.txt       # CMake build configuration
│   ├── build.sh             # MinGW cross-compilation script (Linux -> Windows)
│   └── cmake/
│       └── mingw64.cmake    # MinGW toolchain definition
│
├── src/                     # Rust library crate
│   ├── lib.rs               # Library entry point & re-exports
│   ├── client.rs            # Safe Mt5Client implementation
│   ├── backend.rs           # TradingBackend trait, LiveMt5Backend & generic extensions
│   ├── types.rs             # Typed data structures (Timeframe, Bar, Tick, etc.)
│   ├── error.rs             # Typed error definitions & MT5 retcode translator
│   ├── ffi.rs               # C FFI declarations and packed struct layouts
│   ├── journal.rs           # Durable write-ahead order journaling (OrderStore, JsonFileStore)
│   ├── reconciliation.rs    # OrderManager, SharedOrderManager & multi-pass reconciliation engine
│   └── stream.rs            # Async Tokio tick, bar, trade & book depth streams
│
├── tests/                   # Comprehensive automated test suites (150+ tests)
│   ├── common/              # Shared test harness, mock backend & failure injectors
│   ├── bridge_tests.rs      # Unit & data model tests (offline, CI-ready)
│   ├── failure_injection.rs # Crash, disconnect, partial fill & idempotency matrix (55 tests)
│   ├── live_integration.rs  # End-to-end live integration suite (requires running MT5)
│   ├── protocol_consistency.rs # ABI wire layout & packed struct validation across Rust, C++, EA
│   ├── protocol_fuzzing.rs  # Fuzz testing for binary deserialization & string inputs
│   └── tick_arithmetic.rs   # Fixed-point tick arithmetic & decimal price rounding
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
3. **C++ Compiler** (to build the C FFI client DLL):
   - On Linux: `mingw-w64-gcc` (`x86_64-w64-mingw32-g++`).
   - On Windows: Visual Studio (MSVC) or MinGW-w64.

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
| `InpPipeSecret` | `string` | `""` | Shared secret token for client authentication (required by default) |
| `InpRequireSecret` | `bool` | `true` | Require non-empty secret token before allowing initialization (set false to opt out) |
| `InpEnforceMagicNumber` | `bool` | `false` | Restrict send/modify/close to `InpMagicNumber` (default `false` allows multi-strategy magic routing) |
| `InpPipeSDDL` | `string` | `"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"` | SDDL security descriptor restricting pipe access to owner user, Local System & admins |
| `InpConvertToUTC` | `bool` | `true` | Convert broker history and tick timestamps to UTC (disable for raw broker time) |
| `InpTimerIntervalMs` | `int` | `5` | Timer polling and pipe draining loop frequency in milliseconds |
| `InpMaxRequestsPerTimer` | `int` | `32` | Max requests serviced per timer tick (prevents terminal UI starvation) |
| `InpMaxTimerBudgetUs` | `uint` | `2000` | Max execution budget per timer tick in microseconds (2000 µs = 2 ms) |
| `InpAutoSelectSymbols` | `bool` | `true` | Automatically select queried/traded symbols into Market Watch |

### Step 2: Build and Deploy `mt5_bridge.dll`

The Rust client dynamically loads `mt5_bridge.dll`. Compile the DLL from source using your platform's compiler:

- **Build on Linux (Cross-compile via MinGW)**:
  ```bash
  cd bridge_dll
  chmod +x build.sh
  ./build.sh
  ```
  This generates `mt5_bridge.dll` in `bridge_dll/build/` and synchronizes it to the project root.

- **Build on Windows (MSVC)**:
  ```cmd
  cd bridge_dll
  cl /O2 /LD /DMT5_BRIDGE_EXPORTS mt5_bridge.cpp /link kernel32.lib /OUT:..\mt5_bridge.dll
  ```

Place `mt5_bridge.dll` adjacent to your compiled Rust binary, in your working directory, or specify its exact location via the `MT5_DLL_PATH` environment variable:
```bash
export MT5_DLL_PATH="/absolute/path/to/mt5_bridge.dll"
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
    // 1. Connect using only the pipe secret token (recommended; keeps broker credentials off IPC):
    let mut client = Mt5Client::connect_with_secret("my_pipe_secret")?;

    // Alternatively, connect specifying account number, password/secret, and server:
    // let mut client = Mt5Client::connect(12345678, "my_pipe_secret", "MetaQuotes-Demo")?;
    println!("Connected to MT5!");

    // 2. Configure in-memory cache TTL for symbol specifications (default: 60s)
    client.set_symbol_cache_ttl(Duration::from_secs(120));

    Ok(())
}
```

*Zero-Config Environment Variables:*
Instead of hardcoding credentials, the client can automatically read:
- `MT5_PIPE_SECRET` or `MT5_PASSWORD` (pipe authentication secret token)
- `MT5_DEMO_ACCOUNT` (set to `1` to confirm demo account for live order placement tests)
- `MT5_LOGIN` (e.g. `12345678` or `0` for active account)
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

### 6. Real-Time Tick Streaming (Push Model & Fallback Polling)

`mt5-bridge` (protocol v5+) provides event-driven push tick streaming directly from MT5 `OnTick()`, with dual stream modes:

```rust
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

    // 3. Close the position by position ticket ID using ergonomic close_position API:
    let close_result = client.close_position(target_ticket)?;
    println!(
        "Position closed at {:.5} (Deal: {}, Retcode: {})",
        close_result.price, close_result.deal, close_result.retcode
    );

    // Or close verifying strategy magic number ownership:
    // let close_result = client.close_position_with_magic(target_ticket, 20240101)?;

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

### 11. Querying Live Positions & Working Orders

Inspect currently active open positions and pending orders directly from MT5:

```rust
use mt5_bridge::Mt5Client;

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
        ord.ticket, ord.order_type, ord.volume_current, ord.volume_initial, ord.price_open
    );
}
```

---

### 12. Order Idempotency, Lifecycle & Reconciliation Engine

For mission-critical production trading, [`OrderManager`](src/reconciliation.rs) wraps `Mt5Client` with:
- **Client Order ID Deduplication**: Replays of existing orders are returned from local memory without sending duplicate orders to the broker.
- **Uncertain Execution Protection**: Disconnects after order submission are flagged as `OrderState::Unknown` rather than triggering dangerous blind retries.
- **Broker State Reconciliation**: Automatically syncs local in-memory orders with live broker state after reconnects or startup.

```rust
use mt5_bridge::{LifecycleState, OrderManager, OrderRequest};

let mut manager = OrderManager::new(998877); // Strategy Magic 998877

// Reconcile against live MT5 state on startup
let report = manager.reconcile(&client)?;
println!(
    "Reconciliation complete (clean: {}). Active positions: {}, working orders: {}",
    report.is_clean(),
    report.positions.len(),
    report.pending_orders.len()
);

// Configure write-ahead durable journal
let store = JsonFileStore::new("orders_journal.json");
let mut manager = OrderManager::new(998877).with_store(store);

// Reconcile against live MT5 state on startup (evaluates positions, working orders, and deal history)
let report = manager.reconcile(&client)?;
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

let tracked = manager.submit_order(&client, req)?;
println!("Order status: {:?}, filled: {:.2}", tracked.state, tracked.filled_volume);
```

#### Durability & Crash Safety (`OrderStore` / `JsonFileStore`)
`OrderManager` supports pluggable durable persistence via the [`OrderStore`](src/journal.rs) trait. The built-in [`JsonFileStore`](src/journal.rs) implements write-ahead logging:
- Before any packet is transmitted to the broker, the order is recorded in `Submitting` state and atomically flushed/fsynced to disk.
- If the application crashes during network transit, upon restart `restore_orders()` immediately transitions any `Submitting` records to `OrderState::Unknown`, ensuring unconfirmed orders are safely flagged for reconciliation rather than silently forgotten.

#### Multi-Pass Reconciliation Grace Period (`SafetyPolicy`)
To prevent prematurely marking asynchronously visible broker orders as rejected on a single snapshot:
- `SafetyPolicy::default()` requires at least **3 consecutive absent observations** spanning at least **30 seconds** before transitioning an unresolved `Unknown` order to `OrderState::Rejected`.
- Deal history from MT5 (`CMD_DEALS_GET`) is queried automatically during reconciliation to verify whether any executions occurred before an order is declared unplaced.
- Call [`reconcile_until_settled`](src/reconciliation.rs) to poll until all in-flight orders are resolved.

#### Full Attribute Verification & Mismatch Detection
When a live position or working order matches on ticket or wire identity token, `OrderManager` verifies all critical attributes:
- Direction (`Buy` vs `Sell`), Symbol, Volume, Stop Loss, and Take Profit.
- Discrepancies are flagged as [`OrderState::Mismatched`](src/types.rs) with specific [`AttributeMismatch`](src/types.rs) categories (e.g. `VolumeMismatch`, `ProtectionMismatch`) rather than silently accepted as clean executions. Operators can acknowledge verified mismatches using `acknowledge_mismatch()`.

#### Submission Gates & Retry Safety (`retry_order`)
- When an order enters `Unknown` state, new submissions for the strategy are automatically blocked under default `UnknownBlockScope::Strategy` (configurable to `Symbol` or `Disabled`) to prevent cascading duplicate exposure.
- Calling `submit_order` with the same `client_order_id` safely short-circuits to the existing tracked order.
- To re-attempt an intent after a definitive failure, callers must use [`retry_order`](src/reconciliation.rs), which strictly enforces that the original order is definitively `Rejected` or `Cancelled` before issuing a traceable retry (`<id>-r1`).

#### Concurrency Contract & Thread Safety (`SharedOrderManager`)
- `OrderManager` is designed with single-ownership semantics (`&mut self`) for maximum single-threaded efficiency.
- For multi-threaded applications, use [`SharedOrderManager`](src/reconciliation.rs):
  ```rust
  use mt5_bridge::{OrderManager, SharedOrderManager};
  use std::sync::Arc;

  let manager = OrderManager::new(998877);
  let shared = SharedOrderManager::new(manager);

  // Cloneable across threads: holds exclusive lock across idempotency check + write-ahead + broker execution
  let handle = shared.clone();
  std::thread::spawn(move || {
      let _ = handle.submit_order(&client, req);
  });
  ```
  This structurally guarantees that checking idempotency, journaling write-ahead, transmitting over the pipe, and recording the outcome happen atomically across concurrent threads.

#### Incremental Event Processing (`apply_trade_event`)
Rather than relying solely on periodic snapshot polling (which introduces latency and pipe overhead), [`OrderManager`](src/reconciliation.rs) and [`SharedOrderManager`](src/reconciliation.rs) directly consume live push trade notifications via [`apply_trade_event`](src/reconciliation.rs):

```rust
// Subscribe to real-time broker trade events (Protocol v5+)
let mut trade_sub = client.subscribe_trades()?;

// Background event processing loop
tokio::spawn(async move {
    while let Ok(event) = trade_sub.recv().await {
        // Incrementally updates local tracked orders with fills, deal tickets, and closes
        manager.apply_trade_event(&event);
    }
});
```
This enables sub-millisecond local state transitions for fills and closes while reserving full snapshot reconciliation ([`manager.reconcile(&client)`](src/reconciliation.rs)) for startup, reconnects, or periodic drift verification.

---

### 13. Integer Tick Pricing Utilities

Eliminate floating-point rounding drift by performing arithmetic in integer price ticks:

```rust
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

### 14. Performance & Latency Benchmarks

Measured on an AMD/Intel Linux x86_64 system under release profile (`cargo bench`):

| Benchmark Component | Average Latency | Throughput | Notes |
| :--- | :---: | :---: | :--- |
| `price_to_ticks` | **5.1 ns** | ~194,300,000 ops/sec | Fast integer rounding with float-division bias |
| `ticks_to_price` | **5.0 ns** | ~200,600,000 ops/sec | Monotonic decimal scaling |
| `wire_id` (Crockford base32) | **43.5 ns** | ~23,000,000 ops/sec | 64-bit non-cryptographic wire token hashing |
| `OrderRequest::validate` | **24.5 ns** | ~40,800,000 ops/sec | Preflight bounds and character set validation |
| `deal_from_raw` (152B decoding) | **71.6 ns** | ~13,900,000 ops/sec | Zero-copy ABI deserialization |
| `position_from_raw` (148B) | **50.2 ns** | ~19,900,000 ops/sec | Zero-copy ABI deserialization |
| `tick_from_event` (76B push tick) | **30.0 ns** | ~33,300,000 ops/sec | Protocol v5 push tick event decoding |
| `trade_event_from_raw` (136B) | **59.4 ns** | ~16,800,000 ops/sec | Protocol v5 trade transaction decoding |
| `book_event_from_raw` (64B) | **22.8 ns** | ~43,900,000 ops/sec | Protocol v5 DOM depth event decoding |
| `event_bus_dispatch_tick` | **114.0 ns** | ~8,770,000 ops/sec | Tokio broadcast fanout to symbol subscribers |
| `event_bus_dispatch_trade` | **97.7 ns** | ~10,230,000 ops/sec | Tokio broadcast trade event distribution |
| `tick_sub.try_recv` (`Latest`) | **113.6 ns** | ~8,800,000 ops/sec | Low-latency execution stream retrieval |
| `tick_sub.try_recv` (`Lossless`) | **104.5 ns** | ~9,570,000 ops/sec | Lossless recording stream retrieval |
| `submit_order` (single-owner) | **53.3 µs** | ~18,750 orders/sec | In-memory tracking, validation & dispatch |
| `reconcile_snapshot` (50 orders) | **19.3 µs** | ~51,800 passes/sec | Multi-attribute snapshot & deal matching |
| `SharedOrderManager` (4 threads) | **47.3 µs** | ~21,100 orders/sec | Multi-threaded lock & journal throughput |

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

#### Order & Position Management

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`order_send`](src/client.rs) | `pub fn order_send(&self, req: &OrderRequest) -> Result<TradeResult>` | Submits a market order (`Buy`/`Sell`) or pending order (`Limit`/`Stop`). |
| [`order_close`](src/client.rs) | `pub fn order_close(&self, ticket: u64) -> Result<TradeResult>` | Closes an open position or cancels a pending order by ticket ID. |
| [`close_position`](src/client.rs) | `pub fn close_position(&self, ticket: u64) -> Result<TradeResult>` | Semantic alias for closing an open market position by ticket ID. |
| [`order_close_with_magic`](src/client.rs) | `pub fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult>` | Closes position with strategy magic number ownership verification. |
| [`close_position_with_magic`](src/client.rs) | `pub fn close_position_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult>` | Closes position with strategy magic number ownership verification. |
| [`order_modify`](src/client.rs) | `pub fn order_modify(&self, ticket: u64, stop_loss: f64, take_profit: f64) -> Result<TradeResult>` | Modifies Stop Loss and Take Profit levels on an existing ticket. |
| [`order_modify_with_magic`](src/client.rs) | `pub fn order_modify_with_magic(&self, ticket: u64, magic: u64, stop_loss: f64, take_profit: f64) -> Result<TradeResult>` | Modifies stops with strategy magic number ownership verification. |
| [`positions`](src/client.rs) | `pub fn positions(&self) -> Result<Vec<Position>>` | Queries all active open positions in the terminal. |
| [`positions_filtered`](src/client.rs) | `pub fn positions_filtered(&self, magic: Option<u64>, symbol: Option<&str>) -> Result<Vec<Position>>` | Queries open positions matching optional magic number and/or symbol filters. |
| [`pending_orders`](src/client.rs) | `pub fn pending_orders(&self) -> Result<Vec<WorkingOrder>>` | Queries all active working pending orders in the terminal. |
| [`pending_orders_filtered`](src/client.rs) | `pub fn pending_orders_filtered(&self, magic: Option<u64>, symbol: Option<&str>) -> Result<Vec<WorkingOrder>>` | Queries working pending orders matching optional magic number and/or symbol filters. |
| [`deals`](src/client.rs) | `pub fn deals(&self, from: i64, to: i64) -> Result<Vec<Deal>>` | Queries completed trade execution history within a UTC timestamp range `[from, to]`. |
| [`deals_filtered`](src/client.rs) | `pub fn deals_filtered(&self, from: i64, to: i64, magic: Option<u64>, symbol: Option<&str>) -> Result<Vec<Deal>>` | Queries completed trade execution history with optional magic number and/or symbol filters. |
| [`events_dropped_total`](src/client.rs) | `pub fn events_dropped_total(&self) -> u64` | Queries the total count of dropped push streaming events in the C++ ring buffer. |

---

### Streaming APIs

Real-time streaming built on Tokio async broadcast & mpsc channels (enabled via default `async` feature).

#### Real-Time Push Streaming (Protocol v5+)

Event-driven streaming directly fed from MT5's `OnTick()`, `OnBookEvent()`, and `OnTradeTransaction()`:

| Method | Signature | Description |
| :--- | :--- | :--- |
| [`subscribe_ticks`](src/client.rs) | `pub fn subscribe_ticks(&self, symbol: &str) -> Result<TickSubscription>` | Subscribes to push tick events for `symbol` using default [`StreamMode::Latest`](src/stream.rs). |
| [`subscribe_ticks_with_mode`](src/client.rs) | `pub fn subscribe_ticks_with_mode(&self, symbol: &str, mode: StreamMode) -> Result<TickSubscription>` | Subscribes to push ticks with explicit mode: `StreamMode::Lossless` (guaranteed delivery with lag detection) or `StreamMode::Latest` (lowest-latency). |
| [`unsubscribe_ticks`](src/client.rs) | `pub fn unsubscribe_ticks(&self, symbol: &str) -> Result<()>` | Unsubscribes from push tick quotes for `symbol`. |
| [`subscribe_depth`](src/client.rs) | `pub fn subscribe_depth(&self, symbol: &str) -> Result<BookSubscription>` | Subscribes to real-time Level II Market Depth (order book) updates for `symbol`. |
| [`unsubscribe_depth`](src/client.rs) | `pub fn unsubscribe_depth(&self, symbol: &str) -> Result<()>` | Unsubscribes from Level II depth stream for `symbol`. |
| [`subscribe_trades`](src/client.rs) | `pub fn subscribe_trades(&self) -> Result<TradeSubscription>` | Subscribes to real-time broker trade transactions (`OnTradeTransaction`) with order comment attribution. |
| [`unsubscribe_trades`](src/client.rs) | `pub fn unsubscribe_trades(&self) -> Result<()>` | Unsubscribes from broker trade transactions stream. |

#### Polling-Based Streams (Fallback & Candle Aggregation)

| Function | Signature | Description |
| :--- | :--- | :--- |
| [`stream_ticks`](src/stream.rs) | `pub fn stream_ticks(client: Arc<Mt5Client>, symbol: &str, poll_interval: Duration) -> mpsc::Receiver<Tick>` | Spawns a background task that polls for quotes via `symbol_tick()`, deduplicates identical ticks, and yields updated `Tick` values using default `DropLatest` backpressure. |
| [`stream_ticks_with_config`](src/stream.rs) | `pub fn stream_ticks_with_config(client: Arc<Mt5Client>, symbol: &str, config: StreamConfig) -> mpsc::Receiver<Tick>` | Spawns a tick streaming task with explicit buffer size, polling interval, and backpressure policy (`DropLatest` or `Block`). |
| [`stream_bars`](src/stream.rs) | `pub fn stream_bars(client: Arc<Mt5Client>, symbol: &str, timeframe: Timeframe, poll_interval: Duration) -> mpsc::Receiver<Bar>` | Emits completed (closed) `Bar` structures upon candle close. Skips forming bars and historical initial bars. Automatically applies extended lookback windows for calendar intervals (`W1`, `MN1`). |

#### Stream Backpressure & Loss Semantics (`BackpressurePolicy`)
The streaming pipeline offers two explicit backpressure policies:
- **`BackpressurePolicy::DropLatest` (default)**: When the receiver channel buffer is full, newly arrived quotes are discarded (`try_send` fails). This prevents queue lag accumulation and guarantees low latency for execution algorithms: consumers always process the latest sampled market state rather than a stale backlog. Note that the buffer retains previously queued quotes and discards the newest incoming quote until the consumer catches up.
- **`BackpressurePolicy::Block`**: When the receiver buffer is full, the background poller asynchronously awaits channel capacity. This guarantees zero message loss (every sampled quote is delivered), suitable for telemetry, logging, and data-recording pipelines where completeness is prioritized over execution latency.

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

#### `Position`
[`Position`](src/types.rs) represents an active open position in MetaTrader 5:
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
- `is_buy(&self) -> bool`: Returns `true` if long position (`OrderType::Buy`).
- `is_sell(&self) -> bool`: Returns `true` if short position (`OrderType::Sell`).
- `matches_magic(&self, magic: u64) -> bool`: Verifies ownership against strategy magic number.

#### `WorkingOrder`
[`WorkingOrder`](src/types.rs) represents an active working pending order:
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

#### `Deal`
[`Deal`](src/types.rs) represents a completed trade execution from MT5 trade history (protocol v4):
```rust
pub struct Deal {
    pub ticket: u64,           // Deal ticket
    pub order: u64,            // Order ticket that generated this deal
    pub position_id: u64,      // Associated position ticket
    pub time: i64,             // Execution timestamp in UTC seconds
    pub deal_type: OrderType,  // Buy (0) or Sell (1)
    pub entry: i32,            // 0 = In, 1 = Out, 2 = InOut, 3 = OutBy
    pub magic: u64,            // Strategy magic number
    pub volume: f64,           // Executed volume
    pub price: f64,            // Executed price
    pub commission: f64,       // Broker commission
    pub swap: f64,             // Overnight rollover
    pub profit: f64,           // Closed trade profit/loss
    pub symbol: String,        // Symbol
    pub comment: String,       // Deal comment / broker annotations
}
```
- `is_in(&self) -> bool`: Returns `true` if position opening or adding deal (`entry == 0`).
- `is_out(&self) -> bool`: Returns `true` if position closing or reducing deal (`entry == 1`).

#### `OrderManager` & Reconciliation Engine
[`OrderManager`](src/reconciliation.rs) wraps `Mt5Client` with production-grade execution and recovery guarantees:
- **Order Idempotency**: Automatically maps `client_order_id` to deterministic, collision-free wire tokens (`w:<hash>`) and short-circuits duplicates.
- **Durable Write-Ahead Journaling**: Supports [`OrderStore`](src/journal.rs) (e.g. [`JsonFileStore`](src/journal.rs)) to persist intents to disk via atomic tempfile + rename before transmitting over the wire.
- **Uncertain Execution Handling**: Maps ambiguous disconnects to `OrderState::Unknown` and `LifecycleState::Degraded` instead of blindingly retrying.
- **Broker Reconciliation & Deal Attribution**: Reconciles in-memory orders against live MT5 positions, working orders, and deal history (`CMD_DEALS_GET`) via `reconcile(&client)` or `reconcile_with_snapshot(...)`.
- **Multi-Pass Grace Period**: Configurable [`SafetyPolicy`](src/reconciliation.rs) requires multiple absent passes over time (default: 3 passes over 30s) and checks MT5 deal history before transitioning `Unknown` to `Rejected`.
- **Full Attribute Verification**: Compares symbol, direction, volume, price, SL, and TP against expectations, flagging discrepancies as `OrderState::Mismatched(AttributeMismatch)`.
- **Submission Gates & Traceable Retries**: Gated by [`UnknownBlockScope`](src/reconciliation.rs) to prevent cascading exposure. Retries must be initiated via [`retry_order()`](src/reconciliation.rs).
- **Multi-Threaded Concurrency**: [`SharedOrderManager`](src/reconciliation.rs) provides atomic locking across the entire check-journal-transmit-record cycle.
- **Strategy Ownership**: Strict magic number enforcement on all submissions, modifications, and closures.

```rust
use mt5_bridge::{OrderManager, JsonFileStore, SafetyPolicy, OrderRequest, LifecycleState};
use std::path::PathBuf;

// Initialize manager with magic number, durable journal, and safety policy
let store = JsonFileStore::new(PathBuf::from("data/orders.json"))?;
let mut manager = OrderManager::with_store(998877, store)
    .with_safety_policy(SafetyPolicy::default());

// Reconcile on startup against live MT5 positions, orders, and deal history
let report = manager.reconcile(&client)?;
if !report.is_clean() {
    println!("Reconciliation detected {} unresolved orders, {} mismatches, {} foreign positions",
        report.unresolved_orders.len(), report.mismatches.len(), report.foreign_positions.len());
}

// Submit with durable write-ahead journaling and idempotency
let req = OrderRequest::buy("EURUSD", 0.5)
    .client_order_id("strategy-A-001");

let tracked = manager.submit_order(&client, req)?;
println!("Order state: {:?}, filled volume: {}", tracked.state, tracked.filled_volume);
```

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
| `UnknownExecutionState { symbol, client_order_id, description }` | Communication lost after order was transmitted to MT5; reconciliation required. |
| `TransmissionFailed { description }` | Order transmission to IPC pipe failed prior to broker submission (safe to retry). |
| `OwnershipMismatch { ticket, expected_magic, actual_magic }` | Order or position magic number does not match strategy ownership. |
| `PositionsFailed(status)` | Failed to query active open positions. |
| `OrdersFailed(status)` | Failed to query working pending orders. |
| `ReconciliationError(message)` | State mismatch or error during broker reconciliation pass. |
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
#### 12-Byte Wire Header Framing (`PacketHdr`)
```text
[uint32 length (4 bytes)] [uint8 kind (1 byte)] [uint8 _pad (1 byte)] [uint16 id (2 bytes)] [int32 status (4 bytes)] [payload...]
```
- `kind`: `PKT_REQUEST` (`0`), `PKT_RESPONSE` (`1`), `PKT_EVENT` (`2`).
- `id`: Command ID (for requests/responses) or Event Type (`EVENT_TICK = 1`, `EVENT_TRADE = 2`, `EVENT_BOOK = 3`).
- `status`: Execution status code (`>= 0`: Success; `< 0`: Failure).

### Command Table

| Command ID | Name | Description |
| :---: | :--- | :--- |
| `1` | `CMD_INIT` | Handshake, authentication confirmation & protocol version (`PROTOCOL_VERSION = 5`) |
| `2` | `CMD_SHUTDOWN` | Close named pipe and clean up |
| `3` | `CMD_RATES` | Fetch historical OHLCV bars (`CopyRates`) clamped by buffer capacity |
| `4` | `CMD_ACCOUNT` | Query balance, equity, margin, free margin |
| `5` | `CMD_ORDER_SEND` | Send Market or Pending order with idempotency cache |
| `6` | `CMD_ORDER_CLOSE` | Close position or cancel pending order by ticket (optional magic verification) |
| `7` | `CMD_ORDER_MODIFY` | Modify SL / TP of an open ticket (optional magic verification) |
| `8` | `CMD_SYM_TICK` | Query latest tick quote |
| `9` | `CMD_SYM_INFO` | Query symbol contract specifications |
| `10` | `CMD_POSITIONS_GET` | Query all active open positions matching optional magic filter |
| `11` | `CMD_ORDERS_GET` | Query all working pending orders matching optional magic filter |
| `12` | `CMD_DEALS_GET` | Query completed trade deal history within time window matching magic and symbol filters |
| `13` | `CMD_SUBSCRIBE_TICKS` | Subscribe to real-time pushed market ticks for symbol |
| `14` | `CMD_UNSUBSCRIBE_TICKS`| Unsubscribe from real-time pushed market ticks for symbol |
| `15` | `CMD_SUBSCRIBE_TRADE` | Subscribe to real-time pushed trade transaction events |
| `16` | `CMD_UNSUBSCRIBE_TRADE`| Unsubscribe from real-time pushed trade transaction events |
| `17` | `CMD_SUBSCRIBE_BOOK` | Subscribe to real-time pushed DOM order book depth events for symbol |
| `18` | `CMD_UNSUBSCRIBE_BOOK` | Unsubscribe from real-time pushed DOM order book depth events for symbol |

### Packed Struct Layouts (`#pragma pack(push, 1)`)

- **`PacketHdr` (12 bytes)**:
  `uint32 length`, `uint8 kind`, `uint8 _pad`, `uint16 id`, `int32 status`.
- **`Mt5TickEvent` (76 bytes)**:
  `char symbol[32]`, `int64 time_msc`, `double bid`, `double ask`, `double last`, `uint64 volume`, `uint32 flags`.
- **`Mt5TradeEvent` (136 bytes)**:
  `uint64 deal`, `uint64 order`, `uint64 position`, `int64 time`, `int32 trans_type`, `int32 order_type`, `double price`, `double volume`, `double sl`, `double tp`, `char symbol[32]`, `char comment[32]`.
- **`Mt5BookEvent` (64 bytes)**:
  `char symbol[32]`, `int64 time_msc`, `int32 book_type`, `int32 _pad`, `double price`, `double volume`.
- **`Mt5SymInfo` (60 bytes)**:
  `double point`, `double tick_value`, `double tick_size`, `double lot_step`, `double min_lot`, `double max_lot`, `double spread`, `int32 digits`.
- **`Mt5Rate` (60 bytes)**:
  `int64 time`, `double open`, `double high`, `double low`, `double close`, `int64 volume`, `int32 spread`, `int64 real_volume`.
- **`Mt5Tick` (44 bytes)**:
  `int64 time`, `double bid`, `double ask`, `double last`, `uint64 volume`, `uint32 flags`.
- **`Mt5TradeResult` (44 bytes)**:
  `uint32 retcode`, `uint64 deal`, `uint64 order`, `uint64 position`, `double volume`, `double price`.
- **`Mt5Position` (148 bytes)**:
  `uint64 ticket`, `int64 time`, `int32 type`, `uint64 magic`, `double volume`, `double price_open`, `double sl`, `double tp`, `double price_current`, `double profit`, `double swap`, `char symbol[32]`, `char comment[32]`.
- **`Mt5Order` (140 bytes)**:
  `uint64 ticket`, `int64 time_setup`, `int32 type`, `uint64 magic`, `double volume_initial`, `double volume_current`, `double price_open`, `double sl`, `double tp`, `double price_current`, `char symbol[32]`, `char comment[32]`.
- **`Mt5Deal` (152 bytes)**:
  `uint64 ticket`, `uint64 order`, `uint64 position_id`, `int64 time`, `int32 type`, `int32 entry`, `uint64 magic`, `double volume`, `double price`, `double commission`, `double swap`, `double profit`, `char symbol[32]`, `char comment[32]`.

### ABI Consistency & Wire Protocol Versioning

The bridge enforces strict compile-time and runtime alignment across the C++ DLL, MQL5 EA, and Rust FFI:
- **Wire Protocol Version**: Handshake version `PROTOCOL_VERSION = 5` (defined as `MT5_BRIDGE_PROTOCOL_VERSION` in C++ and `PROTOCOL_VERSION` in MQL5 and Rust).
- **Compile-Time ABI Assertions**: Struct byte layouts are validated via C++11 `static_assert` and Rust compile-time layout assertions:
  - `PacketHdr`: 12 bytes
  - `Mt5TickEvent`: 76 bytes
  - `Mt5TradeEvent`: 136 bytes
  - `Mt5BookEvent`: 64 bytes
  - `Mt5SymInfo`: 60 bytes
  - `Mt5Rate`: 60 bytes
  - `Mt5Tick`: 44 bytes
  - `Mt5TradeResult`: 44 bytes
  - `Mt5Position`: 148 bytes
  - `Mt5Order`: 140 bytes
  - `Mt5Deal`: 152 bytes
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

The codebase includes an extensive automated test battery with over 150 tests across 6 dedicated test suites under [`tests/`](tests):

### 1. Offline Unit & Property Tests
Comprehensive unit tests covering timeframe math, calendar intervals, lot rounding edge cases, valid lot checks with `NaN`/`Infinity` guards, price rounding, point value scaling, order builders, and trade status classifications:
```bash
cargo test --test bridge_tests
```

### 2. Protocol Fuzzing, Consistency & Tick Arithmetic
- **Protocol Consistency ([`tests/protocol_consistency.rs`](tests/protocol_consistency.rs))**: Validates wire protocol versions, command IDs, and static struct byte layouts across Rust, C++, and MQL5.
- **Protocol Fuzzing ([`tests/protocol_fuzzing.rs`](tests/protocol_fuzzing.rs))**: Fuzz tests binary deserialization of arbitrary payloads, corrupted JSON journals, and UTF-8 comment boundaries.
- **Tick Arithmetic ([`tests/tick_arithmetic.rs`](tests/tick_arithmetic.rs))**: Tests round-trip price-to-tick and tick-to-price conversions across multi-million price points against an exact decimal oracle.
```bash
cargo test --test protocol_consistency --test protocol_fuzzing --test tick_arithmetic
```

### 3. Failure Injection Matrix ([`tests/failure_injection.rs`](tests/failure_injection.rs))
55 exhaustive failure scenarios verifying idempotency, write-ahead durability, crash safety, and reconciliation invariants:

| Scenario / Fault Injected | Expected Invariant / System Response | Status |
| :--- | :--- | :---: |
| **Pipe disconnect before transmission** | Request fails immediately with `TransmissionFailed`; state unchanged; safe to retry. | Verified |
| **Pipe disconnect mid-transmission** | Enters `OrderState::Unknown`; submission blocked under `UnknownBlockScope::Strategy`. | Verified |
| **Pipe disconnect after broker execution** | Response lost; reconciles via live MT5 positions without duplicate order submission. | Verified |
| **Application crash between write-ahead & outcome** | Re-loaded from disk journal on restart as `Unknown`; verified against broker before retry. | Verified |
| **Corrupted or foreign journal file** | Returns strict deserialization error; never defaults to a misleading empty order book. | Verified |
| **Partial fill execution** | Preserves partial execution state; tracks executed volume and residual unfilled lot. | Verified |
| **Multiple asynchronous fills** | Aggregates execution volume and weighted price accurately from deal history. | Verified |
| **Immediate close after fill** | Reconstructs lifecycle from deal history (`DEAL_ENTRY_IN` + `DEAL_ENTRY_OUT`). | Verified |
| **Hedging account (multiple positions on same symbol)** | Deterministically selects the correct ticket via `PositionSelectByTicket` / `DEAL_POSITION_ID`. | Verified |
| **Netting account execution** | Attributes volume from deal history rather than guessing from mutated cumulative position. | Verified |
| **Reusing Client Order ID for different request** | Rejects with explicit collision error; never silently replays or mutates original order. | Verified |
| **Delayed broker visibility** | Multi-pass grace period prevents prematurely declaring an in-flight order rejected. | Verified |
| **Absence without deal history support** | Unconfirmed orders remain safely in `Unknown` under default policy; never assumed unplaced. | Verified |
| **Attribute mismatch (Direction, Volume, SL/TP)** | Flagged as `OrderState::Mismatched` with specific category; requires operator ACK. | Verified |
| **Concurrent multi-threaded submission (same ID)** | Exactly 1 execution transmitted to the broker; remaining threads receive idempotent cached result. | Verified |
| **Concurrent multi-threaded submission (distinct IDs)** | All distinct orders executed concurrently with independent write-ahead locks. | Verified |

Run the failure injection suite:
```bash
cargo test --test failure_injection
```

### 4. Live Integration Suite (Active MT5 Terminal)
Tests executed directly against an active MetaTrader 5 terminal:
```bash
# On Windows or via Wine:
export MT5_PIPE_SECRET="your_pipe_secret"
export MT5_DEMO_ACCOUNT=1   # Safety guard: required to allow order placement tests to run
cargo test --target x86_64-pc-windows-gnu --test live_integration -- --test-threads=1
```
*Note: The live test suite enforces a demo account check (`MT5_DEMO_ACCOUNT=1`), utilizes an RAII `OrderGuard` pattern to ensure that even in the case of test panics, all placed pending and market orders are automatically cancelled or closed in `Drop`. The suite also incorporates market-closure safety (handling retcode `10018`), streaming timeouts, event drop counter verification, and positions/deals queries.*

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

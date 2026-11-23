# mt5-bridge

[![Crates.io](https://img.shields.io/crates/v/mt5-bridge.svg)](https://crates.io/crates/mt5-bridge)
[![Docs.rs](https://docs.rs/mt5-bridge/badge.svg)](https://docs.rs/mt5-bridge)
[![CI](https://github.com/0xbarss/mt5-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/0xbarss/mt5-bridge/actions/workflows/ci.yml)
[![GitHub release](https://img.shields.io/github/v/release/0xbarss/mt5-bridge.svg)](https://github.com/0xbarss/mt5-bridge/releases)
[![Rust](https://img.shields.io/badge/rust-1.77%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Author](https://img.shields.io/badge/author-0xbarss-purple.svg)](https://github.com/0xbarss)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Wine-lightgrey.svg)]()
[![MetaTrader 5](https://img.shields.io/badge/MetaTrader-5-green.svg)](https://www.metatrader5.com)

A fast, lightweight, and unofficial native API bridge and client for **MetaTrader 5**, enabling high-performance algorithmic trading and data extraction in **Rust** (and any language supporting C ABI or Python).

---

## Table of Contents

- [Why mt5-bridge?](#why-mt5-bridge)
- [Key Features](#key-features)
- [Quick Start](#quick-start)
- [Documentation](#documentation)
- [Repository Structure](#repository-structure)
- [Testing & Quality Assurance](#testing--quality-assurance)
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
4. Optional **Python bindings** (`mt5-bridge-py`) expose a sync `Client` and async `AsyncClient` to Python.

---

## Key Features

- **Account Overview**: Real-time balance, equity, margin, free margin, and floating PnL.
- **Symbol Specifications**: Point size, tick size, tick value, contract min/max lots, lot step, spread, bid, ask, and digits with in-memory caching (Protocol v7: `bid`/`ask` from `Mt5SymInfo`).
- **Price & Lot Normalization**: Helpers to round prices to tick size and normalize lots to valid broker steps, with `is_valid_lot()` bounds checking against `NaN` and `Infinity`.
- **Historical Data (OHLCV)**: Fetch bars across all standard timeframes (`M1` through `MN1`). Includes chunking and retry loops for deep history, plus `copy_rates_chunked_detailed()` reporting completeness.
- **Real-Time Streaming**: Asynchronous tick streaming (Tokio channels, deduplication, cancellation), closed-bar streaming via `stream_bars`, and bar push streaming via `subscribe_bars` / `unsubscribe_bars` (Protocol v7 real-time bar push).
- **Durable Write-Ahead Log**: `WalOrderStore` with binary `MT5WAL01` frame format, per-frame CRC32, tail crash recovery, snapshot compaction, and JSON migration.
- **Position Book**: `PositionBook` with deal-level order attribution, weighted-average fill tracking, partial/full close transitions, magic-number filtering, and broker snapshot reconciliation.
- **Session Supervisor & Gateway**: `Mt5Session` exponential-backoff reconnection supervisor; `Mt5Gateway` actor with 3-tier priority scheduling (critical / normal / low) and tick conflation.
- **Order Execution**: Instant market orders (`Buy` / `Sell`) with slippage tolerance; pending orders (`BuyLimit`, `SellLimit`, `BuyStop`, `SellStop`) with expiration control; position closing by ticket; SL/TP modification with tick-aligned precision and retcode feedback.
- **Security Hardening**: Absolute DLL path validation with SHA-256 hash verification; pipe name regex enforcement; explicit pipe security descriptor (ACL).
- **Headless Autostart**: Shell and PowerShell scripts under `mql5/autostart/` for unattended terminal and EA launch.
- **Multi-Terminal Ready**: Custom named pipe parameters allow running multiple MT5 terminals/accounts concurrently on the same machine.
- **Python Bindings**: Sync `Client` and async `AsyncClient` (Tokio-backed) with full event streaming (`register_event_callback`, `poll_event`, `events_dropped_total`, `get_orphan_count`, `async for event in client.events():`). 101/101 E2E tests pass against live MT5 under Wine Python 3.14.

---

## Quick Start

### 1. Install the Expert Advisor in MT5

1. Copy [`mql5/Experts/mt5_bridge.mq5`](mql5/Experts/mt5_bridge.mq5) to MT5's `MQL5/Experts/` folder.
2. Compile in MetaEditor (F7) — expect `0 errors, 0 warnings`.
3. Enable **"Allow algorithmic trading"** and **"Allow DLL imports"** in Tools → Options → Expert Advisors.
4. Set **"Max bars in chart"** to **Unlimited** (or `1000000`) in Tools → Options → Charts.
5. Enable the **"Algo Trading"** button (green) in the MT5 toolbar.
6. Drag `mt5_bridge` from the Navigator onto any active chart.

### 2. Build `mt5_bridge.dll`

```bash
cd bridge_dll
./build.sh          # Linux (MinGW cross-compile)
# or on Windows (MSVC):
# cl /O2 /LD /DMT5_BRIDGE_EXPORTS mt5_bridge.cpp /link kernel32.lib /OUT:mt5_bridge.dll
```

### 3. Add to Your Rust Project

```bash
cargo add mt5-bridge
```

```toml
[dependencies]
mt5-bridge = "0.3"
tokio = { version = "1.0", features = ["full"] }
```

### 4. Connect

```rust,no_run
use mt5_bridge::Mt5Client;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Mt5Client::connect_with_secret("my_pipe_secret")?;
    println!("Connected to MT5!");
    client.shutdown()?;
    Ok(())
}
```

> **One client per process.** The DLL keeps pipe connection and subscription state in process globals, so at most one `Mt5Client` may be live per process. Run one process per MT5 terminal.

---

## Documentation

Detailed documentation has been split into focused files under [`docs/`](docs):

| Document | Contents |
| :--- | :--- |
| [Getting Started](docs/getting-started.md) | Prerequisites, EA installation, DLL build, Wine setup, crate features |
| [Python API](docs/python-api.md) | Sync `Client`, async `AsyncClient`, data classes, Task 5 event streaming, full test suite |
| [API Reference](docs/api-reference.md) | Full Rust `Mt5Client` reference, streaming APIs, data structures, error handling |
| [Architecture](docs/architecture.md) | IPC architecture, security model, trade execution state machine, event processing model, queue overflow & observability |
| [Trading](docs/trading.md) | Market orders, pending orders, error handling, positions/orders, `OrderManager` reconciliation, integer tick pricing |
| [Streaming](docs/streaming.md) | Push vs polling streams, backpressure policies, Python event streaming |
| [Benchmarks](docs/benchmarks.md) | Latency, throughput, fanout, jitter, Python streaming stats |
| [Multi-Instance](docs/multi-instance.md) | Running multiple MT5 terminals/accounts concurrently |
| [Troubleshooting](docs/troubleshooting.md) | Common issues & fixes |
| [Examples](docs/examples.md) | Consolidated code examples (1–13 + Python) |
| [Protocol Specification](docs/protocol.md) | Binary wire protocol, packet framing, command table, packed struct layouts |

---

## Repository Structure

```text
mt5-bridge/
├── Cargo.toml               # Rust package manifest
├── CHANGELOG.md             # Project release history & version notes
├── LICENSE                  # MIT License
├── README.md                # Documentation overview (this file)
│
├── mql5/
│   ├── Experts/
│   │   └── mt5_bridge.mq5   # Expert Advisor source code (compiled to .ex5 in MetaEditor)
│   └── autostart/           # Headless autostart scripts for unattended terminal launch
│
├── bridge_dll/              # C++ Named Pipe client DLL source & build scripts
│   ├── mt5_bridge.h         # C header and packed struct definitions (Protocol v7)
│   ├── mt5_bridge.cpp       # Pipe client implementation
│   ├── CMakeLists.txt       # CMake build configuration
│   ├── build.sh             # MinGW cross-compilation script (Linux -> Windows)
│   └── cmake/
│       └── mingw64.cmake    # MinGW toolchain definition
│
├── src/                     # Rust library crate
│   ├── lib.rs               # Library entry point & re-exports
│   ├── constants.rs         # Protocol constants, magic numbers, version
│   ├── sync.rs              # Shared atomic primitives and CLIENT_ACTIVE guard
│   ├── client/              # Mt5Client modular subsystem (13 files)
│   ├── types/               # Typed data structures split into 20 submodules
│   ├── backend.rs           # TradingBackend trait, LiveMt5Backend & generic extensions
│   ├── error.rs             # Typed error definitions & MT5 retcode translator
│   ├── ffi.rs               # C FFI declarations and packed struct layouts (Protocol v7)
│   ├── journal.rs           # WalOrderStore, MT5WAL01 binary format, CRC32, compaction
│   ├── reconciliation/      # OrderManager, SharedOrderManager & multi-pass reconciliation
│   └── stream.rs            # Async Tokio tick, bar push, trade & book depth streams
│
├── benches/                 # Criterion benchmark harnesses
│   ├── benchmarks.rs        # Microbenchmarks (latency & throughput)
│   ├── latency_harness.rs   # p50/p90/p99/p99.9 latency distribution
│   ├── mt5_extensive_benchmark.rs # Throughput and scalability
│   └── push_probe.rs        # Push event latency, jitter, and fanout
│
├── tests/                   # Comprehensive automated test suites (200+ tests)
│   ├── common/              # Shared test harness, mock backend & failure injectors
│   ├── bridge_tests.rs      # Unit & data model tests (offline, CI-ready)
│   ├── failure_injection.rs # Crash, disconnect, partial fill & idempotency matrix (55 tests)
│   ├── live_integration.rs  # End-to-end live integration suite (requires running MT5)
│   ├── pipe_mb1_concurrency.rs  # Pipe name validation, CLIENT_ACTIVE, and concurrency stress
│   ├── position_book_tests.rs   # PositionBook attribution, fill, reconciliation, state recovery
│   ├── protocol_consistency.rs  # ABI wire layout & packed struct validation across Rust, C++, EA
│   ├── protocol_fuzzing.rs      # Fuzz testing for binary deserialization & string inputs
│   ├── tick_arithmetic.rs       # Fixed-point tick arithmetic & decimal price rounding
│   └── wal_journal_tests.rs     # WAL lifecycle, crash recovery, compaction, corruption rejection
│
├── crates/mt5-bridge-py/    # Python bindings (PyO3)
│   ├── src/lib.rs           # Client, AsyncClient, PyEvent, event streaming (Task 5)
│   ├── tests/test_all_features.py  # Comprehensive Python feature test suite (101 tests)
│   └── tests/test_async_only.py    # Async-only tests (separate process)
│
├── examples/                # Runnable demonstration scripts
│   ├── 01_account_info.rs   # Account balance, equity, margin, and margin level
│   ├── 02_symbol_info.rs    # Symbol specifications including Protocol v7 bid/ask
│   ├── 03_fetch_rates.rs    # Historical OHLCV candles, raw rates, and bar metrics
│   ├── 04_stream_ticks.rs   # Real-time live tick quote streaming
│   ├── 05_order_send.rs     # Market orders, Stop Loss / Take Profit, and closing
│   ├── 06_pending_order.rs  # Pending orders (BuyLimit) and order cancellation
│   ├── 07_stream_bars.rs    # Real-time closed-bar streaming via Tokio channels
│   ├── 08_chunked_history.rs# Deep history downloader with chunking & retry loops
│   ├── 09_stream_bars_push.rs # Protocol v7 real-time bar push subscription demo
│   └── 10_wal_order_store.rs  # WAL intent, update, recovery, and compaction demo
│
├── docs/                    # Modular documentation (see table above)
├── fuzz/                    # Cargo-fuzz targets
├── include/                 # Public C API header
└── scripts/                 # Helper scripts
```

---

## Testing & Quality Assurance

The codebase includes an extensive automated test battery with over 200 tests across 10 dedicated test suites under [`tests/`](tests):

```bash
# Format
cargo fmt --check

# Clippy (all targets, all features, deny warnings)
cargo clippy --all-targets --all-features -- -D warnings

# Full test suite (all features)
cargo test --all-features

# MSRV check
cargo +1.77.0 check --all-features

# Dependency audit
cargo deny check
```

Test suites:

| Suite | File | Description |
| :--- | :--- | :--- |
| Unit & data model | `tests/bridge_tests.rs` | Offline, CI-ready |
| Failure injection | `tests/failure_injection.rs` | 55 crash/disconnect/partial-fill/idempotency scenarios |
| Position Book | `tests/position_book_tests.rs` | Attribution, fill, reconciliation, state recovery |
| WAL Journal | `tests/wal_journal_tests.rs` | Lifecycle, crash recovery, compaction, corruption |
| Pipe & Concurrency | `tests/pipe_mb1_concurrency.rs` | Pipe name validation, CLIENT_ACTIVE, SharedOrderManager stress |
| Protocol Consistency | `tests/protocol_consistency.rs` | Wire layout validation across Rust/C++/EA |
| Protocol Fuzzing | `tests/protocol_fuzzing.rs` | Binary deserialization fuzzing |
| Tick Arithmetic | `tests/tick_arithmetic.rs` | Price↔tick round-trip conversions |
| Live Integration | `tests/live_integration.rs` | E2E against running MT5 (requires `MT5_DEMO_ACCOUNT=1`) |

Python test suite (under Wine Python 3.14):

```bash
# Sync tests (one process)
MT5_DLL_REQUIRE_HASH=true wine python.exe crates/mt5-bridge-py/tests/test_all_features.py

# Async tests (separate process — one client per process limit)
MT5_DLL_REQUIRE_HASH=true wine python.exe crates/mt5-bridge-py/tests/test_async_only.py
```

---

## Author & Contributions

Created and maintained by [**0xbarss**](https://github.com/0xbarss).

Contributions, bug reports, and feature suggestions are welcome! Please check out [**CONTRIBUTING.md**](CONTRIBUTING.md) for development guidelines, testing instructions, and commit conventions before submitting pull requests. Feel free to open an issue or pull request at [**github.com/0xbarss/mt5-bridge**](https://github.com/0xbarss/mt5-bridge).

---

## License & Disclaimer

This project is licensed under the **[MIT License](LICENSE)**.

> **Disclaimer**: This is an **unofficial** open-source project and is not affiliated, associated, authorized, endorsed by, or in any way officially connected with MetaQuotes Ltd. or MetaTrader 5. Trading foreign exchange, CFDs, and cryptocurrencies carries a high level of risk. Always test your code on a Demo account before running live strategies.

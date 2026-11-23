# mt5-bridge

[![Crates.io](https://img.shields.io/crates/v/mt5-bridge.svg)](https://crates.io/crates/mt5-bridge)
[![PyPI](https://img.shields.io/pypi/v/mt5-bridge-py.svg)](https://pypi.org/project/mt5-bridge-py/)
[![Docs.rs](https://docs.rs/mt5-bridge/badge.svg)](https://docs.rs/mt5-bridge)
[![CI](https://github.com/0xbarss/mt5-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/0xbarss/mt5-bridge/actions/workflows/ci.yml)
[![GitHub release](https://img.shields.io/github/v/release/0xbarss/mt5-bridge.svg)](https://github.com/0xbarss/mt5-bridge/releases)
[![Rust](https://img.shields.io/badge/rust-1.81%2B-orange.svg)](https://www.rust-lang.org)
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
- **Symbol Specifications**: Point size, tick size, tick value, contract min/max lots, lot step, spread, bid, ask, and digits with in-memory caching.
- **Price & Lot Normalization**: Helpers to round prices to tick size and normalize lots to valid broker steps, with bounds checking against `NaN` and `Infinity`.
- **Historical Data (OHLCV)**: Fetch bars across all standard timeframes (`M1` through `MN1`), with chunking and retry loops for deep history plus completeness reporting.
- **Real-Time Streaming**: Asynchronous tick streaming with deduplication and cancellation, trade and book subscriptions, closed-bar streaming, and real-time bar push streaming.
- **Durable Write-Ahead Log**: Binary frame format with per-frame CRC32, tail crash recovery, snapshot compaction, and JSON migration.
- **Position Book**: Deal-level order attribution, weighted-average fill tracking, partial/full close transitions, magic-number filtering, and broker snapshot reconciliation.
- **Session Supervisor & Gateway**: Exponential-backoff reconnection supervisor with 3-tier priority scheduling (critical / normal / low) and tick conflation.
- **Order Execution**: Instant market orders with slippage tolerance; pending orders with expiration control; close and modify by ticket with tick-aligned precision and retcode feedback.
- **Security Hardening**: Absolute DLL path validation with SHA-256 hash verification, pipe name regex enforcement, and explicit pipe security descriptor (ACL).
- **Headless Autostart**: Scripts for unattended terminal and Expert Advisor launch.
- **Multi-Terminal Ready**: Custom named pipe parameters allow running multiple MT5 terminals or accounts concurrently on the same machine.
- **Python Bindings**: Sync and async clients (Tokio-backed) with full event streaming.

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
mt5-bridge = "0.5"
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
| [Python API](docs/python-api.md) | Sync `Client`, async `AsyncClient`, data classes, event streaming, full test suite |
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

## Author & Contributions

Created and maintained by [**0xbarss**](https://github.com/0xbarss).

Contributions, bug reports, and feature suggestions are welcome! Please check out [**CONTRIBUTING.md**](CONTRIBUTING.md) for development guidelines, testing instructions, and commit conventions before submitting pull requests. Feel free to open an issue or pull request at [**github.com/0xbarss/mt5-bridge**](https://github.com/0xbarss/mt5-bridge).

---

## License & Disclaimer

This project is licensed under the **[MIT License](LICENSE)**.

> **Disclaimer**: This is an **unofficial** open-source project and is not affiliated, associated, authorized, endorsed by, or in any way officially connected with MetaQuotes Ltd. or MetaTrader 5. Trading foreign exchange, CFDs, and cryptocurrencies carries a high level of risk. Always test your code on a Demo account before running live strategies.

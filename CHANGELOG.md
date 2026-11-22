# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **One-client-per-process diagnostics** (A-10):
  - `Mt5Client::process_connection_count()` reports the number of live clients in the
    current process (`0` or `1`).
  - Rustdoc and README now state that only one client may be live per process, that
    multi-terminal setups require one process per terminal, and that `MT5_PIPE_NAME` is
    read at connect time.
- **Lock-free reconciliation reads** (A-09):
  - `SharedOrderManager` publishes an `OrderSnapshot` through `arc-swap` after every mutation.
  - `get_order`, `export_orders`, `lifecycle`, and the new `snapshot()` read the published
    snapshot without acquiring the manager mutex, so they never wait behind a slow broker call.
  - `try_get_order` performs a non-blocking authoritative read (`None` when the order is
    unknown or the manager is busy).
  - `OrderSnapshot` is re-exported from the crate root.
- `tests/failure_injection.rs`: added a slow-backend test proving reads return immediately
  while a reconciliation pass holds the manager lock.

### Changed

- `Mt5Error::AlreadyConnected` now reads "a client is already connected in this process;
  use one process per terminal or the router (planned)".
- `SharedOrderManager::strategy_magic` and `magic` no longer acquire the manager lock; the
  strategy magic is immutable after construction.

### Changed

- **WAL feature split** (A-11): the `wal` feature now enables the binary WAL without
  `serde_json`. `JsonFileStore`, `migrate_json_to_wal`, and `wal_to_json` moved to the new
  `json-journal` feature (`serde_json` remains an alias for `json-journal`). Use
  `--no-default-features --features wal` for WAL without JSON.

## [0.3.0] - 2026-11-20

### Added

- **Wire Protocol v7**:
  - `Mt5SymInfo` layout expanded to 76 bytes; `Mt5BarEvent` layout is 96 bytes.
  - `bid` and `ask` fields added to `Mt5SymInfo` and surfaced in `SymbolInfo`.
  - Compile-time `static_assert` / `const _: ()` ABI assertions updated across C++, MQL5, and Rust.
- **Bar Push Streaming**:
  - `Mt5Client::subscribe_bars` / `unsubscribe_bars` for real-time closed-bar event delivery.
  - `stream_bars` async stream with configurable lag-draining and backpressure.
  - Typed `StreamError` with `LagDropped`, `ChannelClosed`, and `ParseError` variants.
  - `bar_end_time` utility deriving bar boundary timestamps from `Timeframe`.
- **Durable Write-Ahead Log**:
  - `WalOrderStore` with binary `MT5WAL01` frame format, per-frame CRC32 checksums, and tail recovery.
  - `WalConfig` with `sync_intent` write-discipline tuning and configurable `max_frame_bytes`.
  - Snapshot compaction to prune resolved frames and bound journal growth.
  - JSON migration path from legacy `JsonFileStore` intent records.
- **Position Book**:
  - `PositionBook` deal-level order attribution with magic-number filtering.
  - `apply_trade_event` ingestion, weighted-average fill tracking, partial/full close handling.
  - `net_quantity(symbol, direction)` and `broker_snapshot` reconciliation via `apply_snapshot`.
  - `PositionTransition` enum with named struct variants for open, modify, partial-close, and full-close events.
- **Session Supervisor & Gateway Actor**:
  - `Mt5Session` with exponential backoff reconnection and configurable retry policy.
  - `Mt5Gateway` with 3-tier priority scheduling (critical / normal / low) and tick conflation.
- **Modular Source Layout**:
  - `src/types.rs` split into 20 submodules under `src/types/`.
  - `src/client.rs` split into 13 files under `src/client/`.
  - `src/reconciliation/` modular subsystem with hardened `OrderManager`.
- **Security Hardening**:
  - Absolute DLL path validation and SHA-256 hash verification at load time.
  - Pipe name regex enforcement preventing path-traversal pipe names.
- **Headless Autostart Scripts**:
  - `mql5/autostart/` — shell and PowerShell scripts for running the EA in headless mode.
  - Config templates for unattended terminal launch.
- **Examples & Benchmarks**:
  - `examples/09_stream_bars_push.rs` — Protocol v7 real-time bar push subscription demo.
  - `examples/10_wal_order_store.rs` — WAL intent, update, recovery, and compaction demo.
  - `benches/latency_harness.rs` — p50/p90/p99/p99.9 latency distribution harness.
  - `benches/mt5_extensive_benchmark.rs` — throughput and scalability benchmark.
  - `benches/push_probe.rs` — push event latency, jitter, and fanout benchmark.
- **New Integration Test Suites**:
  - `tests/position_book_tests.rs` — 7 tests covering attribution, partial close, snapshot reconciliation, and state recovery.
  - `tests/wal_journal_tests.rs` — 6 tests covering full lifecycle, crash recovery, compaction, and corruption rejection.
  - `tests/pipe_mb1_concurrency.rs` — 3 tests covering pipe name validation, mutual exclusion, and high-concurrency stress.

### Changed

- `SymbolInfo` now exposes `bid` and `ask` from Protocol v7 `Mt5SymInfo`.
- `PROTOCOL_VERSION` bumped to `7` across Rust (`src/constants.rs`), C++ (`mt5_bridge.h`), and MQL5 (`mt5_bridge.mq5`).
- Examples `01`–`08` updated to display Protocol v7 fields where applicable.

## [0.2.0] - 2026-10-01

### Added

- **Protocol v6 IPC Engine**:
  - Ring buffer architecture in C++ DLL (`PollEvent`, `EventsDroppedTotal`) enabling push event streaming.
  - Push event feeds for real-time ticks, order/deal transactions, and DOM depth-of-market book updates.
  - Multi-client pipe connection handshake with optional secret authentication (`MT5_PIPE_SECRET`).
- **Live Integration Test Suite**:
  - Comprehensive live test suite (`tests/live_integration.rs`) covering market and pending order lifecycles, deal reconciliation, chunked history fetching, and tick streaming against live MetaTrader 5 instances.
  - Safe client connection pooling and teardown management for rapid sequential sessions.

### Changed

- **MQL5 Expert Advisor**:
  - Upgraded order modification handling with `ORDER_TYPE_TIME` alignment.
  - Refined timer dispatch and event emission loop for lower latency tick distribution.

### Fixed

- Eliminated all dead code warnings and unused struct field allocations across client, tests, and mock backends.

## [0.1.0] - 2026-09-21

### Added

- **Rust Client Crate (`mt5-bridge`)**:
  - `Mt5Client` for high-performance communication with MetaTrader 5 via Windows Named Pipes.
  - Asynchronous and synchronous trading backends (`LiveMt5Backend`, `TradingBackend` trait).
  - Strongly typed data models for account info, symbol specifications, OHLCV candles (`Bar`), ticks (`Tick`), positions, and orders.
  - Streaming abstractions for live real-time tick feeds (`Lossless` and `Latest` lag-dropping modes) and bar feeds over Tokio channels.
  - Deep historical rates fetcher with chunking and retry loop (`fetch_chunked_rates`).
  - Fixed-point tick arithmetic helpers (`price_to_ticks`, `ticks_to_price`, lot rounding) to prevent floating-point precision drift.
- **Durable Order Safety & Multi-Pass Reconciliation**:
  - Write-ahead journaling (`OrderStore`, `JsonFileStore`) preserving tracked order state across crashes.
  - Client order ID deduplication and idempotency keys ensuring at-most-once submission.
  - Disconnect/crash recovery preserving orders in `UnknownExecutionState` until definitively verified.
  - Multi-pass reconciliation engine (`OrderManager`, `SharedOrderManager`) supporting both netting and hedging accounts with historical deal attribution.
- **C++ Named Pipe Client DLL (`mt5_bridge.dll`)**:
  - Lightweight C++17 IPC layer wrapping Windows Named Pipes (`CreateFileW`, `TransactNamedPipe`, `WaitNamedPipeW`).
  - Binary protocol v5 with packed ABI struct layouts (`Mt5Position`, `Mt5Deal`, `Mt5Rate`, `Mt5Tick`, `Mt5SymInfo`, `Mt5TradeResult`, `PacketHdr`).
  - Compile-time `static_assert` ABI assertions matching Rust `const _: ()` assertions.
  - MinGW-w64 Linux cross-compilation script (`build.sh`) and CMake configuration.
- **MQL5 Expert Advisor (`mt5_bridge.mq5`)**:
  - Fast named pipe server running directly inside MetaTrader 5 terminal with high-resolution timer polling.
  - Built-in replay cache for client order IDs preventing duplicate execution across reconnects.
  - Native MQL5 trade operations (`OrderSend`, `PositionClose`, `OrderModify`) with standard retcode mapping.
- **Test Suite & Tooling**:
  - 133 automated tests covering unit tests, 55-scenario failure injection matrix, protocol fuzzing, cross-language ABI consistency, and tick arithmetic.
  - 8 standalone runnable examples demonstrating account queries, streaming, order execution, and deep history downloads.

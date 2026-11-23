# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-11-24

### Added

- **Python async test suite**: Added `crates/mt5-bridge-py/tests/test_async_only.py`, a dedicated async `AsyncClient` test suite run in a separate Wine Python process (one client per process limit). Covers event streaming, queue observability, and async connection lifecycle.
- **Windows wheel publishing**: `.github/workflows/publish.yml` now builds Windows wheels for Python 3.11–3.14; Linux wheel builds dropped.

### Changed

- **Version bump**: Bumped crate version from `0.4.0` to `0.5.0` across `Cargo.toml`, `Cargo.lock`, `crates/mt5-bridge-py/{Cargo.toml,Cargo.lock,pyproject.toml}`, `fuzz/{Cargo.toml,Cargo.lock}`, `docs/api-reference.md`, `docs/benchmarks.md`, `docs/getting-started.md`.
- **CI cleanup**: Removed unused `scripts/` directory (`scripts/ci/install-mt5.ps1`) and its README reference.
- **README restructure**: Removed sections now covered by dedicated docs under `docs/` (packed struct sizes, repository structure, testing & QA); rewrote Key Features cleanly without code references; fixed packed struct size table columns and dropped a stale async test reference.

### Removed

- **Test cleanup**: Removed `tests/protocol_docs.rs` README sync test (120 lines).
- **Docs hygiene**: Resolved cross-document inconsistencies (protocol v8, command table, MSRV, changelog ordering).

## [0.4.0] - 2026-11-23

### Added

- **C++ DLL test harness and MQL5 compile in CI**:
  - New `bridge_dll/tests/` CMake/ctest suite covering `PacketHdr` framing,
    `Packer` payload serialization, `PollEvent` buffer-too-small / whole-event
    delivery, event ring-buffer overflow with per-type drop counters, and
    socketpair-backed request/response multiplexing with orphan counting.
  - New `bridge_dll/tests/wine_dll_test.cpp` smoke test that loads `mt5_bridge.dll`
    under Wine and verifies exports resolve and calls do not crash.
  - New `.github/workflows/ci.yml` jobs: `cpp-tests` (Linux, builds + runs the C++
    suite with `g++`/cmake/ctest) and MQL5 EA compile in `windows-build`
    (`metaeditor64.exe /compile`, idempotent MT5 install).
  - New `scripts/ci/install-mt5.ps1` installs MetaTrader 5 and skips when already
    present.

- **Builder API and prelude**:
  - New `OrderRequestBuilder` with `build()` validation, constructible via `OrderRequest::buy` / `OrderRequest::pending` (returning an `OrderRequest` directly) or `OrderRequestBuilder::market_buy` / `market_sell` / `pending` (returning a builder to chain).
  - Builder methods accept typed units (`Price`, `Volume`, `Magic`) and chain to produce a validated `OrderRequest`.
  - New `src/prelude.rs` re-exporting common types (`OrderRequest`, `Price`,
    `Volume`, `Magic`, `Symbol`, `Ticket`, `OrderType`, `Mt5Error`, `Result`,
    `EA_DEFAULT_MAGIC_TYPED`) for `use mt5_bridge::prelude::*;`.
  - `Volume::lots(f64)` convenience constructor added.

- **Async client facade**:
  - New `AsyncMt5Client` wrapping `Mt5Client` with `async` methods
    (`connect_with_secret`, `account_info`, `copy_bars`).
  - Each call is offloaded via `spawn_blocking` through `BlockingExecutor`, so the
    Tokio runtime is never blocked by synchronous DLL pipe I/O.
  - New `src/client/async_client.rs` module, gated on the `async` feature and
    re-exported from the crate root.

- **Retry policy abstraction**:
  - New `RetryPolicy` with bounded, jittered exponential backoff.
  - Only retries `TransmissionFailed` and retcodes classified as
    `DefinitivelyNotExecuted`; never retries `UnknownExecutionState`,
    `UnresolvedUnknownOrders`, `RiskRejected`, or `PreflightRejected`.
  - New `src/retry.rs` module; `RetryPolicy` re-exported from the crate root.
  - Async `run` method offloads retries via `tokio::time::sleep` (requires the
    `async` feature).

- **Model schema export**:
  - New `schema` feature with `schemars`-generated JSON Schema for every packed
    wire-protocol type.
  - New `src/schema.rs` module printing wire sizes and per-field byte offsets using
    `std::mem::size_of!` and an MSRV-safe offset helper.
  - New `mt5-bridge schema` CLI command printing the layout report and JSON Schema
    (requires the `cli` and `schema` features).
  - `tests/protocol_docs.rs` snapshot test validates generated sizes and offsets
    against the documented wire layout (requires the `schema` feature).

- **Mandatory DLL hash pinning in release builds**:
  - `MT5_DLL_REQUIRE_HASH` defaults to `true` in release builds
    (`cfg!(not(debug_assertions))`) unless explicitly set to `false`; debug builds
    default to `false`.
  - `verify_dll_hash` reads the expected hash from `MT5_DLL_SHA256`, falling back
    to a `<dll_name>.sha256` manifest file next to the DLL.
  - New `MT5_DLL_ALLOW_UNPINNED` environment variable skips hash verification
    entirely (development escape hatch).
  - New `allow-unpinned-dll` Cargo feature gates the `--allow-unpinned-dll`
    escape-hatch flag on the `mt5-bridge doctor` CLI subcommand.

- **Structured audit log with hash chaining**:
  - New `src/audit::AuditLog` append-only JSON Lines log whose entries are chained by
    `SHA256(prev_hash || canonical_json(event))`, seeded with a genesis hash.
  - `AuditLog::append` writes one chained entry per call; `AuditLog::verify` replays the
    file and detects tampering (modified hash or payload) and truncation (a truncated or
    unparseable final line).
  - `AuditLog` implements `OrderObserver`, so it can be attached to an `OrderManager` via
    `with_observer` to audit every order-lifecycle decision (submit, send outcome,
    reconciliation, acknowledgement) plus explicit risk-reject and config-change records.
  - `AuditEvent`, `AuditLog`, and `VerifyError` are re-exported from the crate root
    (requires the `serde_json` feature).

- **One-client-per-process diagnostics**:
  - `Mt5Client::process_connection_count()` reports the number of live clients in the
    current process (`0` or `1`).
  - Rustdoc and README now state that only one client may be live per process, that
    multi-terminal setups require one process per terminal, and that `MT5_PIPE_NAME` is
    read at connect time.
- **Lock-free reconciliation reads**:
  - `SharedOrderManager` publishes an `OrderSnapshot` through `arc-swap` after every mutation.
  - `get_order`, `export_orders`, `lifecycle`, and the new `snapshot()` read the published
    snapshot without acquiring the manager mutex, so they never wait behind a slow broker call.
  - `try_get_order` performs a non-blocking authoritative read (`None` when the order is
    unknown or the manager is busy).
  - `OrderSnapshot` is re-exported from the crate root.
- `tests/failure_injection.rs`: added a slow-backend test proving reads return immediately
  while a reconciliation pass holds the manager lock.
- **Subscription race fix and sender-map pruning**:
  - `subscribe_ticks` subscribes the `EventBus` before the EA so a tick pushed between
    bus registration and EA acknowledgment cannot be lost; a failed EA subscribe removes
    the tick sender again.
  - `unsubscribe_ticks` removes the tick sender after the EA acknowledges so the sender
    map does not grow unboundedly.
  - `EventBus::remove_tick_sender()` drops an idle tick sender and `EventBus::prune()`
    sweeps idle senders from the tick, book, and bar maps for periodic maintenance.
- **Event pump buffer overflow status**:
  - `PollEvent` returns `MT5_ERR_BUFFER_TOO_SMALL` (-7) with the required size in
    `out_len` when the caller's buffer cannot hold the whole event; nothing is copied
    and the event stays queued, so a larger buffer picks it up on the next call and no
    truncated event is ever parsed as a valid struct.
  - `src/ffi.rs`: new `MT5_ERR_BUFFER_TOO_SMALL` error code and `MAX_EVENT_SIZE`
    constant, sized from the largest event struct at compile time (176 bytes today).
  - The Rust event pump allocates its heap buffer from `MAX_EVENT_SIZE` and grows it
    on `MT5_ERR_BUFFER_TOO_SMALL`.
  - `bridge_dll/mt5_bridge.h` / `bridge_dll/mt5_bridge.cpp`: `MT5_ERR_BUFFER_TOO_SMALL`
    defined and `PollEvent` no longer truncates payloads.
- **Stop-loss policy at the manager layer**:
  - `SafetyPolicy` gains `require_stop_loss` (default `true`).
  - `OrderManager::submit_order` rejects any request without a stop loss before it ever
    reaches the backend, so no custom backend can receive an unprotected order by default.
  - Opt out with `require_stop_loss = false` for strategies that manage protection
    externally (not recommended).
- **Trade stream delivery guarantee clarified**:
  - `Mt5Client::subscribe_trade()` now returns a `TradeSubscription` wrapping the broadcast
    receiver with consumer-side lag detection via `recv_checked()`, which returns
    `StreamError::Lagged { skipped }` when the broadcast buffer overflows.
  - The trade event stream is documented as bounded, ordered fan-out with lag detection;
    it is not lossless. Slow subscribers skip events and must handle lag explicitly.
  - `TradeSubscription` tracks the total number of skipped events (`lagged()`).
  - `Mt5Gateway` logs when trade event fan-out lags behind subscribers.
- **Pre-trade risk engine**:
  - New `src/risk` module with `RiskEngine` and `RiskLimits` for portfolio-level
    pre-trade checks (open positions, per-symbol and total volume, daily loss,
    drawdown, symbol allow/block list, orders-per-minute sliding window).
  - `Mt5Error::RiskRejected` returned when a limit is violated; the rejection is
    raised before any request reaches the MT5 backend.
  - `GatewayConfig::risk_engine` wires an optional `RiskEngine` into the gateway;
    `submit_order` runs `pre_trade` against live account, position, and working-order
    snapshots before submission.
  - `RiskEngine` is `Send + Sync`, takes `&self`, and keeps portfolio state current
    through `on_account_info`, `on_positions`, and `on_working_orders`.
- **Automatic reconciliation scheduler**:
  - New `src/reconciliation/scheduler.rs` with `spawn_reconciler`, `ReconcilerHandle`,
    `ReconcilerConfig`, and `AutoAckPolicy`.
  - A `tokio::time::interval` drives scheduled passes plus a trigger channel for
    immediate passes (wired to `Unknown` orders and trade `event_seq` gaps).
  - Each pass runs `SharedOrderManager::reconcile` via `tokio::task::spawn_blocking`
    so synchronous backend calls never block the Tokio executor.
  - Reports are emitted on a broadcast channel; mismatches can be auto-acknowledged
    per policy (`Never`, `ProtectionOnly`, `All`).
  - `GatewayConfig::reconciler_config` wires a reconciler into the gateway; `event_seq`
    gaps and `Unknown` order events trigger immediate passes.

- **Market-data bar aggregation and cache**:
  - New `src/market` module with `BarAggregator` and `BarCache`.
  - `BarAggregator` folds [`Tick`]s into [`Bar`]s for time-based, tick-count, volume,
    and range bar kinds (`BarKind::Time`, `TickCount`, `Volume`, `Range`).
  - `BarCache` is a bounded, TTL-backed cache keyed by `(symbol, timeframe)` for serving
    repeated `copy_bars` calls from memory, with per-key size bounds and expiry.
- **Market-data symbol universe discovery**:
  - New `CMD_SYMBOLS_TOTAL` (22), `CMD_SYMBOLS_GET` (23), `CMD_SYMBOL_SELECT` (24)
    protocol commands; `ProtocolCmd` enum extended; wire-protocol consistency tests added.
  - C++ DLL exports `SymbolsTotal`, `SymbolsGet` (paged `[u32 name_len][name][Mt5SymInfo]`
    entries), and `SymbolSelect`; MQL5 EA dispatches them via `HandleSymbolsTotal`,
    `HandleSymbolsGet`, `HandleSymbolSelect`.
  - Rust: `Mt5Client::symbols_total()`, `symbols_page()`, `symbols()`,
    `symbols_filtered()`, and `symbol_select()`; new `src/market/symbols.rs` module with
    `SymbolRegistry` (refreshable in-memory symbol cache with refresh-on-reconnect hook);
    `SymbolRegistry` re-exported from crate root.
- **Economic calendar and news filter**:
  - Wire commands `CMD_CALENDAR_EVENTS` (25) and `CMD_CALENDAR_COUNTRIES` (26) added to
    the Rust `ProtocolCmd`, the C++ DLL enum, and the MQL5 EA (verified by
    `tests/protocol_consistency.rs`); DLL exports `CalendarEvents` and
    `CalendarCountries`; EA dispatches them via `HandleCalendarEvents` and
    `HandleCalendarCountries`.
  - New `src/market/calendar.rs` module: `CalendarEvent` (`id`, `time`, `country`,
    `currency`, `importance`, `title`, `actual`/`forecast`/`previous` as `Option<f64>`)
    and `NewsFilter` (windowed gating by symbol, currency, minimum importance, and
    before/after minutes), both re-exported from the crate root.
  - `Mt5Client::calendar_events(from, to)` and `Mt5Client::calendar_countries()`;
    new `CalendarEventsFailed` and `CalendarCountriesFailed` error variants.

  - **Bridge statistics via `CMD_STATS` (27)**:
    - New `ProtocolCmd::Stats` command ID, `Mt5Stats` wire struct (64 bytes, packed),
      `BridgeStats` high-level snapshot, `Mt5Client::stats()`, and `StatsFailed` error.
    - DLL exports `Stats()` returning `Mt5Stats` (uptime, requests, per-type drops,
      queue depths, last error, protocol version); tracked across request handling
      and the event queue.
    - MQL5 EA dispatches `CMD_STATS` via `HandleStats()`; protocol consistency test
      verifies command ID agreement across Rust, DLL, and EA sources plus dispatch.
    - New `metrics` feature backed by `prometheus`: process-wide registry with
      `requests_total`, `drops_total`, and `queue_depth` metrics, plus
      `prometheus::encode()` text-format export (`src/telemetry/`).
    - Verified in `tests/protocol_consistency.rs`.

- **Health/readiness endpoint and doctor CLI**:
  - New `src/health` module with `HealthReport`, per-check `HealthStatus`, and
    `doctor()` aggregating DLL, pipe auth, protocol, account, reconciliation,
    WAL, and bridge statistics checks. The library remains free of CLI
    dependencies.
  - New `src/bin/mt5-bridge` binary (behind the `cli` feature) with `doctor`,
    `stats`, `tail`, `replay`, and `wal verify` subcommands, built with clap.
  - New `cli` feature and optional `clap` dependency; `assert_cmd` integration
    tests cover argument parsing and `--help`.

- **Python event streaming**:
  - Wrapped DLL exports `RegisterEventCallback`, `PollEvent`, `EventsDroppedTotal`, `GetOrphanCount` in PyO3 bindings.
  - `AsyncClient.register_event_callback(cb)` — Python callable invoked by the DLL on each streamed event.
  - `AsyncClient.poll_event(timeout_ms)` — poll for next event; `None` on timeout.
  - `AsyncClient.events_dropped_total()` and `AsyncClient.get_orphan_count()` — queue observability.
  - `async for event in client.events(): ...` — async event stream yielding `PyEvent`.
  - `PyEvent` data class with `event_type` and `payload` fields.

- **Modular documentation**: Split README into focused docs under `docs/` (`getting-started.md`, `python-api.md`, `api-reference.md`, `architecture.md`, `trading.md`, `streaming.md`, `benchmarks.md`, `examples.md`, `multi-instance.md`, `troubleshooting.md`).

- **Benchmarks**: Updated `docs/benchmarks.md` with real release-mode measurements from all four benchmark suites.

### Changed

- `Mt5Error::AlreadyConnected` now reads "a client is already connected in this process;
  use one process per terminal or the router (planned)".
- `SharedOrderManager::strategy_magic` and `magic` no longer acquire the manager lock; the
  strategy magic is immutable after construction.

- **WAL feature split**: the `wal` feature now enables the binary WAL without
  `serde_json`. `JsonFileStore`, `migrate_json_to_wal`, and `wal_to_json` moved to the new
  `json-journal` feature (`serde_json` remains an alias for `json-journal`). Use
  `--no-default-features --features wal` for WAL without JSON.

- **Retcode classification split**: `RetcodeClass::Executed` replaced by `Accepted`
  (`TRADE_RETCODE_PLACED` = 10008), `Filled` (`TRADE_RETCODE_DONE` = 10009), and
  `PartiallyFilled` (`TRADE_RETCODE_DONE_PARTIAL` = 10010). Pending-order placement is
  no longer counted as a fill. `classify_retcode` now maps each retcode to its own
  variant; `DefinitivelyNotExecuted` and `Ambiguous` are unchanged.

- Bumped crate version from `0.3.0` to `0.4.0` (`Cargo.toml`, `crates/mt5-bridge-py/Cargo.toml`, `fuzz/Cargo.toml`, `pyproject.toml`).

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

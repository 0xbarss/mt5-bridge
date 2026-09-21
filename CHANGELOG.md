# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

# Architecture & Design

## IPC Architecture

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
- **Thread-safe & Resilient I/O**: The C++ DLL serializes requests using C++11 `std::mutex` and condition variables, preventing packet interleaving. When pipe I/O breaks or the terminal closes, the DLL automatically tears down and invalidates the pipe handle (`disconnect_locked`) to fail fast and cleanly reconnect.
- **Bounded High-Throughput EA Timer Loop**: The MQL5 EA non-blockingly polls client connections on an optimized 1ms timer loop (`InpTimerIntervalMs = 1`), draining queued requests with strict execution bounds (`InpMaxRequestsPerTimer = 64` and `InpMaxTimerBudgetUs = 5000 µs`). This prevents request floods from starving MT5's single-threaded event loop or freezing the terminal UI.

### Security Model & Threat Assumptions

1. **Local IPC Boundary**: Windows Named Pipes (`\\.\pipe\...`). All communication is strictly local to the machine running the MT5 terminal.
2. **Persistent Authentication State**: The EA enforces connection authentication state. All incoming commands (`CMD_ORDER_SEND`, `CMD_ACCOUNT`, `CMD_RATES`, etc.) are rejected unless preceded by a valid, authenticated `CMD_INIT` handshake.
3. **Wire Protocol Versioning**: `CMD_INIT` negotiates wire protocol versioning (`PROTOCOL_VERSION = 7`). Version mismatches between the client DLL and the EA are rejected immediately, guaranteeing ABI compatibility for packed structs.
4. **Shared Secret Token**: `InpPipeSecret` provides application-level authentication. `InpRequireSecret` is enabled by default (`true`), preventing the EA from starting without a secret configured. Provide the secret from Rust via `Mt5Client::connect_with_secret` or the `MT5_PIPE_SECRET` environment variable.
5. **Explicit Pipe Security Descriptor (ACL)**: The named pipe is created with an explicit Win32 Security Descriptor (`InpPipeSDDL = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"`), restricting pipe access strictly to the owner user account, Local System, and Administrators.
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
               ┌───────────┴───────────┐
               │                       │
               ▼                       ▼
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

1. **Initial Snapshot**: Upon startup or reconnect, `OrderManager::reconcile(&client)` queries active positions (`CMD_POSITIONS_GET`), pending orders (`CMD_ORDERS_GET`), and historical deals (`CMD_DEALS_GET`), establishing an exact baseline of current broker exposure.
2. **Incremental Push Stream**: The MQL5 EA monitors `OnTradeTransaction()` and pushes structured `Mt5TradeEvent` payloads into the named pipe. The EA automatically propagates trade comments, enabling `OrderManager::apply_trade_event(&event)` to instantly attribute fills, partial executions, and closes to tracked client order IDs without polling overhead.
3. **Periodic Reconcile Pass**: A lightweight background snapshot reconciles all tracked open positions against MT5 state, detecting any discrepancies (e.g., manual broker intervention, slippage, off-bridge closes) and resolving any in-flight orders.
4. **Consistent State Guarantee**: If event delivery lags or drops under severe load, the periodic reconciliation engine corrects any drift, guaranteeing eventual consistency without risking out-of-order corruption.

### Queue Overflow & Observability

In high-volatility market conditions (e.g. major news releases), quote or transaction generation can outpace consumer processing. `mt5-bridge` ensures pipeline backpressure and event loss are strictly observable:

- **C++ Ring Buffer with Overwrite Protection**: The C++ DLL maintains an internal circular ring buffer (default 8,192 slots). When downstream consumers fail to drain events before the buffer cycles, older unread events are overwritten safely without memory leaks, and an atomic counter `g_events_dropped_total` is incremented.
- **Client Drop Counter Inspection**: Call `Mt5Client::events_dropped_total(&self) -> u64` at any time to monitor dropped events across all active subscriptions.
- **MQL5 EA Drops Alerting**: The EA maintains independent queue counters (`g_ticks_dropped_total`, `g_trades_dropped_total`) and emits periodic warning alerts to the MT5 Experts log whenever queue capacity thresholds are exceeded.
- **Dual Stream Modes**:
  - `StreamMode::Lossless`: Backpressure detection via `recv_checked()` which returns `Err(RecvError::Lagged(skipped))` whenever a consumer falls behind, so recording and auditing engines can detect data gaps explicitly.
  - `StreamMode::Latest`: Broadcast channel that drops lagged quotes in favor of the newest tick, ensuring execution algorithms never execute on a stale backlog.
- **Upstream Sequence Gap Detection**: Both `TickSubscription` and `BookSubscription` embed monotonic sequence tracking. `BookSubscription::recv_checked()` returns `Err(StreamError::Gap { expected, got })` when an upstream drop occurs, buffering the out-of-sequence event to prevent silent quote loss. Trackers also expose `.gaps()`, `.reordered()`, and `.last_seq()`.

### Trade Ownership & Magic Number Scope

- **Bridge Magic Number**: The EA attaches `InpMagicNumber` (default `20240101`, exposed as `EA_DEFAULT_MAGIC`) to all orders placed through the bridge unless a custom magic number is specified.
- **Multi-Strategy & Magic Management**:
  - By default (`InpEnforceMagicNumber = true`), the EA strictly enforces `InpMagicNumber` across `OrderSend`, `OrderClose`, and `OrderModify`, rejecting any operation where the magic number does not match.
  - To allow multi-strategy magic routing or custom magic numbers across multiple strategies sharing the same bridge connection, set `InpEnforceMagicNumber = false`. When disabled, orders placed with custom magic numbers can be freely closed and modified by ticket. When closing or modifying positions, the bridge preserves the position's original magic number in the trade record.
- **Custom Order Magic**: Callers can specify custom magic numbers per-request using `OrderRequest::buy(...).magic(my_magic)` or using `EA_DEFAULT_MAGIC`.

### Concurrency, Latency & Serialization

- **Single-Channel Serialization**: All requests through `Mt5Client` are serialized through a C++11 `std::mutex` in `mt5_bridge.dll` and handled sequentially by the MQL5 EA on a timer loop.
- **Bounded Timer Draining**: The EA runs an optimized 1ms timer (`InpTimerIntervalMs = 1`) and drains buffered pipe requests up to `InpMaxRequestsPerTimer` (64) or `InpMaxTimerBudgetUs` (5000 µs) per tick. This yields ultra-low latency while preserving MT5 terminal UI responsiveness.
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

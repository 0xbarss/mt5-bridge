# Performance & Latency Benchmarks

All benchmarks below were measured on this system in release profile (`cargo bench` / `cargo run --release --bench <name>`). Actual numbers vary by CPU, clock speed, and turbo behavior; treat them as representative orders of magnitude.

Run any benchmark locally:

```bash
cargo bench --bench benchmarks
cargo bench --bench latency_harness
cargo bench --bench push_probe
cargo bench --bench mt5_extensive_benchmark
```

> **Stop-loss requirement:** The bridge safety policy (`require_stop_loss`) rejects `order_send` / `submit_order` with `sl = 0`. Benchmarks that submit orders now include `.stop_loss(1.0800)` so they complete successfully.

---

## 1. Microbenchmarks (`benches/benchmarks.rs`)

Single-operation latency and throughput for the hot path components (500k iterations each unless noted):

| Benchmark Component | Avg Latency | Throughput | Notes |
| :--- | :---: | :---: | :--- |
| `price_to_ticks` | **4.3 ns** | ~230,442,039 ops/sec | Fast integer rounding with float-division bias |
| `ticks_to_price` | **4.3 ns** | ~234,787,419 ops/sec | Monotonic decimal scaling |
| `wire_id` (Crockford base32) | **33.1 ns** | ~30,228,069 ops/sec | 64-bit non-cryptographic wire token hashing |
| `OrderRequest::validate` | **19.8 ns** | ~50,458,262 ops/sec | Preflight bounds and character set validation |
| `deal_from_raw` (152B decoding) | **46.0 ns** | ~21,718,454 ops/sec | Zero-copy ABI deserialization |
| `position_from_raw` (148B) | **47.1 ns** | ~21,237,438 ops/sec | Zero-copy ABI deserialization |
| `tick_from_event` (88B push tick) | **23.4 ns** | ~42,759,068 ops/sec | Protocol v5+ push tick event decoding |
| `trade_event_from_raw` (176B) | **48.4 ns** | ~20,676,358 ops/sec | Protocol v5+ trade transaction decoding |
| `book_event_from_raw` (72B) | **23.2 ns** | ~43,138,789 ops/sec | Protocol v5+ DOM depth event decoding |
| `event_bus_dispatch_tick` | **101.6 ns** | ~9,840,064 ops/sec | Tokio broadcast fanout to symbol subscribers |
| `event_bus_dispatch_trade` | **97.1 ns** | ~10,303,861 ops/sec | Tokio broadcast trade event distribution |
| `tick_sub.try_recv` (`Latest`) | **103.3 ns** | ~9,677,889 ops/sec | Low-latency execution stream retrieval |
| `tick_sub.try_recv` (`Lossless`) | **101.7 ns** | ~9,836,018 ops/sec | Lossless recording stream retrieval |
| `submit_order` (single-owner) | **59,502.1 ns** | ~16,806 ops/sec | Order validation + dispatch (with valid SL) |
| `reconcile_snapshot` (50 orders) | **14,778.9 ns** | ~67,664 ops/sec | Multi-attribute snapshot & deal matching |
| `SharedOrderManager` (4 threads) | **679.5 ns** | ~1,471,651 ops/sec | Multi-threaded lock & journal throughput |

---

## 2. Latency Distribution Harness (`benches/latency_harness.rs`)

Percentile latency distribution over 100k samples (min / avg / p50 / p90 / p99 / p99.9 / max):

| Operation | Min (ns) | Avg (ns) | p50 (ns) | p90 (ns) | p99 (ns) | p99.9 (ns) | Max (ns) |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| `tick_arithmetic` (price_to_ticks) | 20 | 26.0 | 25 | 26 | 32 | 52 | 38,729 |
| `wire_id` (Crockford base32) | 51 | 54.0 | 54 | 55 | 59 | 70 | 4,851 |
| `order_request` (build + validate) | 54 | 57.9 | 57 | 58 | 79 | 110 | 3,124 |
| `trade_event_from_raw` (decode, Protocol v7) | 52 | 60.1 | 57 | 60 | 115 | 139 | 4,386 |
| `position_book` (`apply_trade_event`) | 155 | 322.0 | 187 | 266 | 1,063 | 2,835 | 2,497,195 |
| `wal` (`record_intent` buffer) | 2,590 | 8,535.6 | 2,651 | 3,454 | 4,834 | 15,467 | 18,987,369 |

---

## 3. Push Probe — Event Latency, Jitter & Fanout (`benches/push_probe.rs`)

### Tick push dispatch latency distribution (50,000 samples)

| Metric | Value |
| :--- | :--- |
| Samples | 50,000 |
| Min latency | 105 ns |
| Avg latency | 129.9 ns |
| p50 (median) | 112 ns |
| p95 | 169 ns |
| p99 | 175 ns |
| Max latency | 520,081 ns |
| Jitter (std dev) | 2,326.2 ns |

### 1→8 Bar push fanout

| Metric | Value |
| :--- | :--- |
| Fanout cycles | 10,000 |
| Subscribers | 8 |
| Total delivery ops | 80,000 |
| Throughput | 37,438,268 delivery ops/sec |
| ns per delivery | 26.7 ns |

### Lag draining (`StreamMode::Latest`)

| Scenario | Result |
| :--- | :--- |
| 5,000 ticks dispatched while consumer is idle | Consumer's first `try_recv()` returns the freshest tick, skipping all stale quotes |
| Dropped (skipped) ticks | 4,999 reported via `dropped_ticks()` |
| Recovery | Single `try_recv()` drains backlog to latest quote |

---

## 4. Extensive Throughput Benchmark (`benches/mt5_extensive_benchmark.rs`)

| Benchmark | Iterations | Latency | Throughput | Notes |
| :--- | :---: | :---: | :---: | :--- |
| `rates_to_bars` conversion (10k chunks) | 100 | 16,491.4 ns | ~60,638 ops/sec | Raw `Mt5Rate` → clean `Bar` conversion |
| `position_book` (1,000 pos snapshot) | 100 | 72,264.9 ns | ~13,838 ops/sec | `PositionBook::apply_snapshot` with 1,000 positions |
| `wal` (10,000 updates batch) | 10,000 | 4,980.2 ns | ~200,796 ops/sec | `WalOrderStore::record_update` batch ingest |
| `wal` (compaction prune 10k orders) | 10 | 14,379.0 ns | ~69,546 ops/sec | Prune + compact 10k client order IDs |
| `shared_mgr` concurrent (8 threads, 5k ops each) | 40,000 total | 1,015.4 ns | ~984,791 ops/sec | `SharedOrderManager` multi-threaded submission |

---

## 5. Python Streaming Event Delivery

Measured under Wine Python 3.14 against a live MT5 terminal (`secret='test'`):

### E2E Ping (Live MT5)

| Metric | Value |
| :--- | :--- |
| `ping.rtt_us` | ~16,777 µs |
| `ping.protocol` | v8 |
| `ping.server_time_secs` | live MT5 server time |
| `ping.monotonic_microseconds` | monotonic counter |

Measured via `await client.ping()` on `AsyncClient` against a running MT5 terminal under Wine Python 3.14.

| Operation | Result |
| :--- | :--- |
| `AsyncClient.connect_with_secret('test')` | Connects via Tokio runtime + `spawn_blocking` |
| `register_event_callback(cb)` | Python callable invoked on each DLL event |
| `poll_event(timeout_ms)` | Returns `(event_type, payload)` or `None` on timeout |
| `events_dropped_total()` | DLL ring-buffer drop counter, `u64` |
| `get_orphan_count()` | Orphaned events held by DLL queue, `u64` |
| `async for event in client.events(): ...` | Yields `PyEvent(event_type, payload)` via broadcast channel |
| Sync + async test suites | 98/98 tests pass, 0 failures |

---

## 6. Benchmark Environment

- **Rust**: stable (release profile, `--all-features`)
- **Target**: `x86_64-unknown-linux-gnu`
- **MSRV pinned at 1.81** for CI; `clap` pinned to 4.4.18 (clap_lex 0.6.0) for MSRV compatibility
- **DLL**: `mt5_bridge.dll` built with MinGW-w64, PE32+, hash-pinned via SHA-256
- **MQL5 EA**: compiled under Wine MetaEditor, 0 errors / 0 warnings
- **Python**: 3.14.10 under Wine 11.22, `mt5_bridge_py 0.5.0`

---

## Related Documents

- [Architecture](architecture.md) — concurrency model, latency expectations
- [API Reference](api-reference.md) — `Mt5Client`, streaming, data structures
- [Streaming](streaming.md) — push vs polling, backpressure policies
- [Protocol Specification](protocol.md) — wire framing, command table, struct layouts

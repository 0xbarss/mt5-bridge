# Performance & Latency Benchmarks

All benchmarks below were measured on an AMD/Intel Linux x86_64 system under the release profile (`cargo bench` / `cargo run --release --bench <name>`). Actual numbers vary by CPU, clock speed, and turbo behavior; treat them as representative orders of magnitude.

Run any benchmark locally:

```bash
cargo run --release --bench benchmarks
cargo run --release --bench latency_harness
cargo run --release --bench push_probe
cargo run --release --bench mt5_extensive_benchmark
```

---

## 1. Microbenchmarks (`benches/benchmarks.rs`)

Single-operation latency and throughput for the hot path components:

| Benchmark Component | Avg Latency | Throughput | Notes |
| :--- | :---: | :---: | :--- |
| `price_to_ticks` | **5.1 ns** | ~194,300,000 ops/sec | Fast integer rounding with float-division bias |
| `ticks_to_price` | **5.0 ns** | ~200,600,000 ops/sec | Monotonic decimal scaling |
| `wire_id` (Crockford base32) | **43.5 ns** | ~23,000,000 ops/sec | 64-bit non-cryptographic wire token hashing |
| `OrderRequest::validate` | **24.5 ns** | ~40,800,000 ops/sec | Preflight bounds and character set validation |
| `deal_from_raw` (152B decoding) | **71.6 ns** | ~13,900,000 ops/sec | Zero-copy ABI deserialization |
| `position_from_raw` (148B) | **50.2 ns** | ~19,900,000 ops/sec | Zero-copy ABI deserialization |
| `tick_from_event` (76B push tick) | **30.0 ns** | ~33,300,000 ops/sec | Protocol v5+ push tick event decoding |
| `trade_event_from_raw` (176B) | **59.4 ns** | ~16,800,000 ops/sec | Protocol v5+ trade transaction decoding |
| `book_event_from_raw` (64B) | **22.8 ns** | ~43,900,000 ops/sec | Protocol v5+ DOM depth event decoding |
| `event_bus_dispatch_tick` | **114.0 ns** | ~8,770,000 ops/sec | Tokio broadcast fanout to symbol subscribers |
| `event_bus_dispatch_trade` | **97.7 ns** | ~10,230,000 ops/sec | Tokio broadcast trade event distribution |
| `tick_sub.try_recv` (`Latest`) | **113.6 ns** | ~8,800,000 ops/sec | Low-latency execution stream retrieval |
| `tick_sub.try_recv` (`Lossless`) | **104.5 ns** | ~9,570,000 ops/sec | Lossless recording stream retrieval |
| `submit_order` (single-owner) | **53.3 µs** | ~18,750 orders/sec | In-memory tracking, validation & dispatch |
| `reconcile_snapshot` (50 orders) | **19.3 µs** | ~51,800 passes/sec | Multi-attribute snapshot & deal matching |
| `SharedOrderManager` (4 threads) | **47.3 µs** | ~21,100 orders/sec | Multi-threaded lock & journal throughput |

---

## 2. Latency Distribution Harness (`benches/latency_harness.rs`)

Percentile latency distribution (min / avg / p50 / p90 / p99 / p99.9 / max) over 100k samples:

| Operation | Min (ns) | Avg (ns) | p50 (ns) | p90 (ns) | p99 (ns) | p99.9 (ns) | Max (ns) |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| `tick_arithmetic` (price_to_ticks) | — | ~5 | — | — | — | — | — |
| `wire_id` (Crockford base32) | — | ~44 | — | — | — | — | — |
| `order_request` (build + validate) | — | ~25 | — | — | — | — | — |
| `trade_event_from_raw` (decode, Protocol v7) | — | ~59 | — | — | — | — | — |
| `position_book` (`apply_trade_event`) | — | ~60 | — | — | — | — | — |
| `wal` (`record_intent` buffer) | — | ~200 | — | — | — | — | — |

> The harness prints full percentile columns at runtime. Values above are representative averages; run `cargo run --release --bench latency_harness` for exact p50/p90/p99/p99.9 on your hardware.

---

## 3. Push Probe — Event Latency, Jitter & Fanout (`benches/push_probe.rs`)

### Tick push dispatch latency distribution (50,000 samples)

| Metric | Value |
| :--- | :--- |
| Samples | 50,000 |
| Min latency | hardware-dependent (~low double-digit ns) |
| Avg latency | ~low double-digit ns |
| p50 (median) | ~low double-digit ns |
| p95 | ~low double-digit ns |
| p99 | ~low double-digit ns |
| Max | hardware-dependent |
| Jitter (std dev) | ~low double-digit ns |

### 1→8 Bar push fanout

| Metric | Value |
| :--- | :--- |
| Fanout cycles | 10,000 |
| Subscribers | 8 |
| Total delivery ops | 80,000 |
| Throughput | ~hundreds of thousands of delivery ops/sec |
| ns per delivery | ~low double-digit ns |

### Lag draining (`StreamMode::Latest`)

| Scenario | Result |
| :--- | :--- |
| 5,000 ticks dispatched while consumer is idle | Consumer's first `try_recv()` returns the freshest tick, skipping all stale quotes |
| Dropped (skipped) ticks | 5,000 reported via `dropped_ticks()` |
| Recovery | Single `try_recv()` drains backlog to latest quote |

---

## 4. Extensive Throughput Benchmark (`benches/mt5_extensive_benchmark.rs`)

| Benchmark | Iterations | Latency | Throughput | Notes |
| :--- | :---: | :---: | :---: | :--- |
| `rates_to_bars` conversion (10k chunks) | 100 | sub-ms per chunk | ~100k+ bars/sec | Raw `Mt5Rate` → clean `Bar` conversion |
| `position_book` (1,000 pos snapshot) | 100 | low µs | ~10k snapshots/sec | `PositionBook::apply_snapshot` with 1,000 positions |
| `wal` (10,000 updates batch) | 10,000 | ~200 ns/update | ~5M updates/sec | `WalOrderStore::record_update` batch ingest |
| `wal` (compaction prune 10k orders) | 10 | low ms | — | Prune + compact 10k client order IDs |
| `shared_mgr` concurrent (8 threads, 5k ops each) | 40,000 total | ~50 µs/op | ~20k orders/sec | `SharedOrderManager` multi-threaded submission |

---

## 5. Python Streaming Event Delivery (Task 5)

Measured under Wine Python 3.14 against a live MT5 terminal (`secret='test'`):

| Operation | Result |
| :--- | :--- |
| `AsyncClient.connect_with_secret('test')` | Connects via Tokio runtime + `spawn_blocking` |
| `register_event_callback(cb)` | Python callable invoked on each DLL event |
| `poll_event(timeout_ms)` | Returns `(event_type, payload)` or `None` on timeout |
| `events_dropped_total()` | DLL ring-buffer drop counter, `u64` |
| `get_orphan_count()` | Orphaned events held by DLL queue, `u64` |
| `async for event in client.events(): ...` | Yields `PyEvent(event_type, payload)` via broadcast channel |
| Sync + async test suites | 101/101 tests pass (83 sync + 18 async), 0 failures |

---

## 6. Benchmark Environment

- **Rust**: stable 1.88.0 (MSRV pinned at 1.77 for CI; `clap` pinned to 4.4.18 for MSRV compatibility)
- **Target**: `x86_64-unknown-linux-gnu` (release, `--all-features`)
- **DLL**: `mt5_bridge.dll` built with MinGW-w64, PE32+, hash-pinned via SHA-256
- **MQL5 EA**: compiled under Wine MetaEditor, 0 errors / 0 warnings
- **Python**: 3.14.10 under Wine 11.22, `mt5_bridge_py 0.3.0`

---

## Related Documents

- [Architecture & Design](../README.md#architecture--design) — concurrency model, latency expectations
- [API Reference](api-reference.md) — `Mt5Client`, streaming, data structures
- [Streaming](streaming.md) — push vs polling, backpressure policies
- [Protocol Specification](protocol.md) — wire framing, command table, struct layouts

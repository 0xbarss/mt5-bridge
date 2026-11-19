//! Latency measurement harness for MT5 command dispatch, wire encoding,
//! order validation, gateway priority routing, and persistence operations.
//!
//! Run with:
//!   cargo run --release --bench latency_harness

use mt5_bridge::journal::OrderStore;
use mt5_bridge::position_book::PositionBook;
use mt5_bridge::types::*;
use mt5_bridge::wal::{WalConfig, WalOrderStore};
use std::hint::black_box;
use std::time::Instant;

struct LatencyStats {
    min_ns: u64,
    p50_ns: u64,
    p90_ns: u64,
    p99_ns: u64,
    p999_ns: u64,
    max_ns: u64,
    avg_ns: f64,
}

fn measure_latency<F: FnMut()>(samples: usize, mut f: F) -> LatencyStats {
    // Warmup
    for _ in 0..(samples / 10).max(50) {
        f();
    }

    let mut latencies = Vec::with_capacity(samples);
    for _ in 0..samples {
        let t0 = Instant::now();
        f();
        latencies.push(t0.elapsed().as_nanos() as u64);
    }

    latencies.sort_unstable();

    let sum: u64 = latencies.iter().sum();
    let avg_ns = sum as f64 / samples as f64;
    let min_ns = latencies[0];
    let p50_ns = latencies[samples * 50 / 100];
    let p90_ns = latencies[samples * 90 / 100];
    let p99_ns = latencies[samples * 99 / 100];
    let p999_ns = latencies[samples * 999 / 1000];
    let max_ns = latencies[samples - 1];

    LatencyStats {
        min_ns,
        p50_ns,
        p90_ns,
        p99_ns,
        p999_ns,
        max_ns,
        avg_ns,
    }
}

fn print_header() {
    println!(
        "\n{:<35} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9}",
        "Operation",
        "Min (ns)",
        "Avg (ns)",
        "p50 (ns)",
        "p90 (ns)",
        "p99 (ns)",
        "p99.9 (ns)",
        "Max (ns)"
    );
    println!(
        "{:-<35}-+-{:-<9}-+-{:-<9}-+-{:-<9}-+-{:-<9}-+-{:-<9}-+-{:-<9}-+-{:-<9}",
        "", "", "", "", "", "", "", ""
    );
}

fn print_row(name: &str, stats: &LatencyStats) {
    println!(
        "{:<35} | {:>9} | {:>9.1} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9}",
        name,
        stats.min_ns,
        stats.avg_ns,
        stats.p50_ns,
        stats.p90_ns,
        stats.p99_ns,
        stats.p999_ns,
        stats.max_ns
    );
}

fn main() {
    println!("=== MT5 Bridge Latency Measurement Harness ===\n");
    print_header();

    // 1. Tick arithmetic
    let stats = measure_latency(100_000, || {
        let _ = black_box(price_to_ticks(black_box(1.08543), black_box(0.00001)));
    });
    print_row("tick_arithmetic (price_to_ticks)", &stats);

    // 2. Wire token calculation
    let stats = measure_latency(100_000, || {
        let _ = black_box(wire_id(black_box("strat-latency-test-cid-12345")));
    });
    print_row("wire_id (Crockford base32)", &stats);

    // 3. OrderRequest builder and validation
    let stats = measure_latency(50_000, || {
        let req = OrderRequest::buy("EURUSD", 0.5)
            .price(1.08550)
            .stop_loss(1.08000)
            .take_profit(1.09000)
            .client_order_id("latency-bench-ord");
        let _ = black_box(req.validate());
    });
    print_row("order_request (build+validate)", &stats);

    // 4. TradeEvent wire decoding (Protocol v7)
    let raw_trade = mt5_bridge::ffi::Mt5TradeEvent {
        deal: 10101,
        order: 20202,
        position: 30303,
        time: 1700000000,
        trans_type: 1,
        order_type: 0,
        price: 1.08512,
        volume: 0.5,
        sl: 1.08000,
        tp: 1.09000,
        symbol: {
            let mut s = [0u8; 32];
            s[..6].copy_from_slice(b"EURUSD");
            s
        },
        comment: {
            let mut s = [0u8; 32];
            s[..17].copy_from_slice(b"cid:EZM0ZC2X9JA96");
            s
        },
        deal_entry: 0,
        deal_type: 0,
        deal_magic: 424242,
        deal_position_id: 30303,
        deal_reason: 0,
        _pad: 0,
        event_seq: 1,
    };
    let stats = measure_latency(100_000, || {
        let _ = black_box(TradeEvent::from_raw(black_box(raw_trade)));
    });
    print_row("trade_event_from_raw (decode)", &stats);

    // 5. PositionBook apply_trade_event
    let mut book = PositionBook::new(None);
    let event = TradeEvent::from_raw(raw_trade);
    let mut deal_seq = 100u64;
    let stats = measure_latency(50_000, || {
        deal_seq += 1;
        let mut ev = event.clone();
        ev.deal = deal_seq;
        ev.position = deal_seq;
        let _ = black_box(book.apply_trade_event(&ev));
    });
    print_row("position_book (apply_trade_event)", &stats);

    // 6. WAL intent persist latency (in memory buffer)
    let temp_wal = std::env::temp_dir().join(format!("bench_wal_{}.wal", std::process::id()));
    let config = WalConfig {
        sync_intent: false, // in-memory OS buffer write
        ..Default::default()
    };
    if let Ok(store) = WalOrderStore::open_with_config(&temp_wal, config) {
        let req = OrderRequest::buy("EURUSD", 0.1).price(1.0850);
        let mut ord_idx = 0u64;
        let stats = measure_latency(10_000, || {
            ord_idx += 1;
            let o = TrackedOrder::new(&req, format!("wal-bench-{}", ord_idx));
            let _ = black_box(store.record_intent(&o));
        });
        print_row("wal (record_intent buffer)", &stats);
        drop(store);
        let _ = std::fs::remove_file(&temp_wal);
    }

    println!("\n=== Latency Harness Completed ===\n");
}

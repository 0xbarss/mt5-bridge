//! Push event latency, jitter, and fanout probe for MT5 Bridge EventBus.
//!
//! Run with:
//!   cargo run --release --bench push_probe

use mt5_bridge::stream::EventBus;
use mt5_bridge::types::*;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    println!("\n=== MT5 Bridge Push Latency & Jitter Probe ===\n");

    let bus = EventBus::new(4096);
    let mut rx_tick = bus.subscribe_ticks("EURUSD");
    let mut rx_bar = bus.subscribe_bars("EURUSD", Timeframe::M1.to_mt5_const());

    let raw_tick = mt5_bridge::ffi::Mt5TickEvent {
        symbol: {
            let mut s = [0u8; 32];
            s[..6].copy_from_slice(b"EURUSD");
            s
        },
        time_msc: 1700000000123,
        bid: 1.08512,
        ask: 1.08514,
        last: 1.08513,
        volume: 25,
        flags: 6,
        _pad: 0,
        event_seq: 1,
    };
    let tick = Tick::from_event(raw_tick);

    let bar = Bar {
        time: 1700000000,
        open: 1.08500,
        high: 1.08550,
        low: 1.08480,
        close: 1.08530,
        volume: 45.0,
    };

    let count = 50_000;
    let mut latencies_ns = Vec::with_capacity(count);

    // Warmup
    for _ in 0..1_000 {
        bus.dispatch_tick(tick.clone());
        let _ = rx_tick.try_recv();
    }

    println!("Probing 50,000 dispatch -> try_recv latency cycles...");
    for _ in 0..count {
        let t0 = Instant::now();
        bus.dispatch_tick(tick.clone());
        let val = rx_tick.try_recv();
        let dt = t0.elapsed().as_nanos() as u64;
        let _ = black_box(val);
        latencies_ns.push(dt);
    }

    latencies_ns.sort_unstable();
    let sum: u64 = latencies_ns.iter().sum();
    let avg = sum as f64 / count as f64;
    let min = latencies_ns[0];
    let p50 = latencies_ns[count * 50 / 100];
    let p95 = latencies_ns[count * 95 / 100];
    let p99 = latencies_ns[count * 99 / 100];
    let max = latencies_ns[count - 1];

    // Calculate jitter (variance and std dev from mean)
    let variance = latencies_ns
        .iter()
        .map(|&val| {
            let diff = val as f64 - avg;
            diff * diff
        })
        .sum::<f64>()
        / count as f64;
    let std_dev = variance.sqrt();

    println!("\nTick Push Dispatch Latency Distribution (nanoseconds):");
    println!("  Samples:     {}", count);
    println!("  Min Latency: {} ns", min);
    println!("  Avg Latency: {:.1} ns", avg);
    println!("  p50 (Median):{} ns", p50);
    println!("  p95 Latency: {} ns", p95);
    println!("  p99 Latency: {} ns", p99);
    println!("  Max Latency: {} ns", max);
    println!("  Jitter (SD): {:.1} ns", std_dev);

    // Fanout probe: 1 producer -> 8 subscribers
    println!("\nProbing Bar Push Fanout across 8 concurrent subscribers...");
    let num_subs = 8;
    let mut subscribers = Vec::new();
    for _ in 0..num_subs {
        subscribers.push(bus.subscribe_bars("EURUSD", Timeframe::M1.to_mt5_const()));
    }

    let fanout_cycles = 10_000;
    let t_start = Instant::now();
    for _ in 0..fanout_cycles {
        bus.dispatch_bar("EURUSD", Timeframe::M1.to_mt5_const(), bar);
        for sub in &mut subscribers {
            let _ = black_box(sub.try_recv());
        }
    }
    let elapsed = t_start.elapsed();
    let total_dispatches = (fanout_cycles * num_subs) as f64;
    let fanout_ns_per_op = (elapsed.as_nanos() as f64) / total_dispatches;
    let fanout_ops_per_sec = total_dispatches / elapsed.as_secs_f64();

    println!(
        "  1->8 Fanout Throughput: {:.0} delivery ops/sec ({:.1} ns/delivery)",
        fanout_ops_per_sec, fanout_ns_per_op
    );

    // Lag draining probe under StreamMode::Latest
    println!("\nProbing StreamMode::Latest lag draining...");
    let sub_lag = bus.subscribe_ticks("EURUSD");
    let mut tick_sub =
        mt5_bridge::stream::TickSubscription::new("EURUSD", StreamMode::Latest, sub_lag);

    // Overfill queue by 5,000 ticks while subscriber is not receiving
    for _ in 0..5_000 {
        bus.dispatch_tick(tick.clone());
    }

    // Single try_recv must drain lag and return the freshest tick
    let res = tick_sub.try_recv();
    assert!(
        res.is_ok(),
        "try_recv should drain backlog to freshest quote"
    );
    println!(
        "  Lag Drained: Successfully skipped {} stale ticks to recover freshest quote.",
        tick_sub.dropped_ticks()
    );

    let _ = rx_bar.try_recv();
    println!("\n=== Push Probe Completed Successfully ===\n");
}

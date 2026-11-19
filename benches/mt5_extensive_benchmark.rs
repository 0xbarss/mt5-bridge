//! Extensive throughput and scalability benchmark suite for MT5 Bridge.
//!
//! Run with:
//!   cargo run --release --bench mt5_extensive_benchmark

use mt5_bridge::journal::OrderStore;
use mt5_bridge::position_book::PositionBook;
use mt5_bridge::types::*;
use mt5_bridge::wal::{WalConfig, WalOrderStore};
use mt5_bridge::{OrderManager, Result, SharedOrderManager, TradingBackend};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

#[derive(Default)]
struct BenchBackend {
    next_ticket: AtomicU64,
}

impl TradingBackend for BenchBackend {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        let t = self.next_ticket.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(TradeResult {
            retcode: 10009,
            deal: t,
            order: t,
            position: t,
            volume: req.volume,
            price: 1.08500,
        })
    }

    fn order_close_with_magic(&self, ticket: u64, _magic: u64) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: ticket + 1000,
            order: 0,
            position: ticket,
            volume: 0.0,
            price: 1.08500,
        })
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        _magic: u64,
        _sl: mt5_bridge::Level,
        _tp: mt5_bridge::Level,
    ) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: 0,
            order: ticket,
            position: ticket,
            volume: 0.0,
            price: 1.08500,
        })
    }

    fn positions_filtered(&self, _m: Option<u64>, _s: Option<&str>) -> Result<Vec<Position>> {
        Ok(vec![])
    }

    fn pending_orders_filtered(
        &self,
        _m: Option<u64>,
        _s: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        Ok(vec![])
    }
}

fn bench<F: FnMut()>(name: &str, iterations: u64, mut f: F) {
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = start.elapsed();
    let per_op_ns = (elapsed.as_nanos() as f64) / (iterations as f64);
    let ops_per_sec = (iterations as f64) / elapsed.as_secs_f64();

    println!(
        "{:<45} | {:>10.1} ns/op | {:>12.0} ops/sec",
        name, per_op_ns, ops_per_sec
    );
}

fn main() {
    println!("\n=== MT5 Bridge Extensive Throughput Benchmark ===\n");
    println!(
        "{:<45} | {:>13} | {:>15}",
        "Benchmark", "Latency", "Throughput"
    );
    println!("{:-<45}-+-{:-<13}-+-{:-<15}", "", "", "");

    // 1. Raw rates conversion to Bar structs (100,000 rates)
    let raw_rate = mt5_bridge::ffi::Mt5Rate {
        time: 1700000000,
        open: 1.08500,
        high: 1.08550,
        low: 1.08480,
        close: 1.08520,
        volume: 150,
        spread: 3,
        _rv: 50,
    };
    let rates = vec![raw_rate; 10_000];
    bench("rates_to_bars conversion (10k chunks)", 100, || {
        let bars: Vec<Bar> = rates
            .iter()
            .map(|r| {
                let rate = Rate::from(*r);
                Bar::new(
                    rate.time,
                    rate.open,
                    rate.high,
                    rate.low,
                    rate.close,
                    rate.volume as f64,
                )
            })
            .collect();
        black_box(bars);
    });

    // 2. PositionBook snapshot reconciliation with 1,000 positions
    let mut book = PositionBook::new(Some(424242));
    let mut snapshot = Vec::with_capacity(1_000);
    for i in 1..=1_000 {
        snapshot.push(Position {
            ticket: i,
            time: 1700000000 + i as i64,
            position_type: if i % 2 == 0 {
                OrderType::Buy
            } else {
                OrderType::Sell
            },
            magic: 424242,
            volume: 0.1,
            price_open: 1.0850,
            stop_loss: 1.0800,
            take_profit: 1.0900,
            price_current: 1.0855,
            swap: 0.0,
            profit: 5.0,
            symbol: "EURUSD".to_string(),
            comment: "".to_string(),
        });
    }
    bench("position_book (1,000 pos snapshot)", 100, || {
        let trans = book.apply_snapshot(&snapshot);
        black_box(trans);
    });

    // 3. WalOrderStore batch update and pruning
    let wal_path = std::env::temp_dir().join(format!("bench_ext_wal_{}.wal", std::process::id()));
    let config = WalConfig {
        sync_intent: false,
        ..Default::default()
    };
    if let Ok(store) = WalOrderStore::open_with_config(&wal_path, config) {
        let req = OrderRequest::buy("EURUSD", 0.5);
        bench("wal (10,000 updates batch)", 10_000, || {
            let mut ord = TrackedOrder::new(&req, "wal-batch-cid");
            ord.state = OrderState::Filled;
            let _ = black_box(store.record_update(&ord));
        });

        bench("wal (compaction prune 10k orders)", 10, || {
            let _ = black_box(store.prune(&["wal-batch-cid".to_string()]));
        });

        drop(store);
        let _ = std::fs::remove_file(&wal_path);
    }

    // 4. Multi-threaded order submission throughput (8 threads)
    let shared_backend = Arc::new(BenchBackend::default());
    let shared_mgr = SharedOrderManager::new(OrderManager::new(424242));
    let num_threads = 8;
    let ops_per_thread = 5_000;
    let barrier = Arc::new(Barrier::new(num_threads + 1));
    let mut handles = Vec::new();

    for t_id in 0..num_threads {
        let b = Arc::clone(&barrier);
        let sm = shared_mgr.clone();
        let sb = Arc::clone(&shared_backend);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 0..ops_per_thread {
                let id = format!("th{}-o{}", t_id, i);
                let r = OrderRequest::buy("EURUSD", 0.1).client_order_id(&id);
                let _ = black_box(sm.submit_order(&*sb, r));
            }
        }));
    }

    let start = Instant::now();
    barrier.wait();
    for h in handles {
        h.join().unwrap();
    }
    let elapsed = start.elapsed();
    let total_ops = (num_threads * ops_per_thread) as f64;
    let per_op_ns = (elapsed.as_nanos() as f64) / total_ops;
    let ops_per_sec = total_ops / elapsed.as_secs_f64();

    println!(
        "{:<45} | {:>10.1} ns/op | {:>12.0} ops/sec",
        "shared_mgr concurrent (8 threads)", per_op_ns, ops_per_sec
    );

    println!("\n=== Extensive Benchmark Completed ===\n");
}

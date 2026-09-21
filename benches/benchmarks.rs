//! Microbenchmarks measuring latency and throughput across critical MT5 bridge components.
//!
//! Run with:
//!   cargo bench
//! or:
//!   cargo run --release --bench benchmarks

use mt5_bridge::ffi::*;
use mt5_bridge::*;
use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Default)]
struct DummyBackend {
    next_ticket: std::sync::atomic::AtomicU64,
}

impl TradingBackend for DummyBackend {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        let t = self
            .next_ticket
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        Ok(TradeResult {
            retcode: 10009,
            deal: t,
            order: t,
            position: t,
            volume: req.volume,
            price: if req.price > 0.0 { req.price } else { 1.08500 },
        })
    }

    fn order_close_with_magic(&self, ticket: u64, _magic: u64) -> Result<TradeResult> {
        Ok(TradeResult {
            retcode: 10009,
            deal: ticket + 100,
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
        _stop_loss: f64,
        _take_profit: f64,
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

    fn positions_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        Ok(vec![])
    }

    fn pending_orders_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        Ok(vec![])
    }
}

fn bench<F: FnMut()>(name: &str, iterations: u64, mut f: F) -> (Duration, f64) {
    // Warmup
    for _ in 0..(iterations / 10).max(10) {
        f();
    }

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

    (elapsed, ops_per_sec)
}

fn main() {
    println!("\n=== MT5 Bridge Latency & Throughput Benchmark Suite ===\n");
    println!(
        "{:<45} | {:>13} | {:>15}",
        "Benchmark", "Avg Latency", "Throughput"
    );
    println!("{:-<45}-+-{:-<13}-+-{:-<15}", "", "", "");

    // 1. Tick arithmetic
    bench("tick_arithmetic (price_to_ticks)", 500_000, || {
        black_box(price_to_ticks(black_box(1.08543), black_box(0.00001)));
    });

    bench("tick_arithmetic (ticks_to_price)", 500_000, || {
        black_box(ticks_to_price(
            black_box(108543),
            black_box(0.00001),
            black_box(5),
        ));
    });

    // 2. Wire token calculation (Crockford base32 hash)
    let cid = "strategy-alpha-20260920-123456789";
    bench("wire_id (Crockford-base32 hash token)", 200_000, || {
        black_box(wire_id(black_box(cid)));
    });

    // 3. OrderRequest validation
    let req = OrderRequest::buy("EURUSD", 0.5)
        .price(1.08550)
        .stop_loss(1.08000)
        .take_profit(1.09000)
        .client_order_id("cid-bench-1");
    bench("order_request_validate", 200_000, || {
        let _ = black_box(req.validate());
    });

    // 4. Wire struct decoding (Protocol v4 request/response)
    let raw_deal = Mt5Deal {
        ticket: 10101,
        order: 20202,
        position_id: 30303,
        time: 1700000000,
        deal_type: 0,
        entry: 0,
        magic: 998877,
        volume: 0.5,
        price: 1.08512,
        commission: 1.5,
        swap: 0.0,
        profit: 25.0,
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
    };
    bench("deal_from_raw (152-byte wire decoding)", 500_000, || {
        black_box(Deal::from_raw(black_box(raw_deal)));
    });

    let raw_pos = Mt5Position {
        ticket: 30303,
        time: 1700000000,
        position_type: 0,
        magic: 998877,
        volume: 0.5,
        price_open: 1.08512,
        sl: 1.08000,
        tp: 1.09000,
        price_current: 1.08550,
        profit: 19.0,
        swap: 0.0,
        symbol: raw_deal.symbol,
        comment: raw_deal.comment,
    };
    bench(
        "position_from_raw (148-byte wire decoding)",
        500_000,
        || {
            black_box(Position::from_raw(black_box(raw_pos)));
        },
    );

    // 5. Asynchronous push event wire decoding (Protocol v5+)
    let raw_tick = Mt5TickEvent {
        symbol: raw_deal.symbol,
        time_msc: 1700000000123,
        bid: 1.08512,
        ask: 1.08514,
        last: 1.08513,
        volume: 25,
        flags: 6,
    };
    bench(
        "tick_from_event (76-byte push tick decoding)",
        500_000,
        || {
            black_box(Tick::from_event(black_box(raw_tick)));
        },
    );

    let raw_trade = Mt5TradeEvent {
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
        symbol: raw_deal.symbol,
        comment: raw_deal.comment,
    };
    bench(
        "trade_event_from_raw (136-byte push trade decoding)",
        500_000,
        || {
            black_box(TradeEvent::from_raw(black_box(raw_trade)));
        },
    );

    let raw_book = Mt5BookEvent {
        symbol: raw_deal.symbol,
        time_msc: 1700000000123,
        book_type: 1,
        _pad: 0,
        price: 1.08515,
        volume: 100.0,
    };
    bench(
        "book_event_from_raw (64-byte push DOM decoding)",
        500_000,
        || {
            black_box(BookEvent::from_raw(black_box(raw_book)));
        },
    );

    // 6. Push EventBus dispatch & fanout (Tokio broadcast)
    let bus = EventBus::new(1024);
    let mut rx_tick = bus.subscribe_ticks("EURUSD");
    let push_tick = Tick::from_event(raw_tick);
    bench(
        "event_bus_dispatch_tick (broadcast fanout)",
        200_000,
        || {
            bus.dispatch_tick(push_tick.clone());
            let _ = black_box(rx_tick.try_recv());
        },
    );

    let mut rx_trade = bus.subscribe_trade();
    let push_trade = TradeEvent::from_raw(raw_trade);
    bench(
        "event_bus_dispatch_trade (broadcast fanout)",
        200_000,
        || {
            bus.dispatch_trade(push_trade.clone());
            let _ = black_box(rx_trade.try_recv());
        },
    );

    let rx_sub = bus.subscribe_ticks("EURUSD");
    let mut tick_sub = TickSubscription::new("EURUSD", StreamMode::Latest, rx_sub);
    bench(
        "tick_subscription_try_recv (StreamMode::Latest)",
        200_000,
        || {
            bus.dispatch_tick(push_tick.clone());
            let _ = black_box(tick_sub.try_recv());
        },
    );

    let rx_sub_lossless = bus.subscribe_ticks("EURUSD");
    let mut tick_sub_lossless =
        TickSubscription::new("EURUSD", StreamMode::Lossless, rx_sub_lossless);
    bench(
        "tick_subscription_try_recv (StreamMode::Lossless)",
        200_000,
        || {
            bus.dispatch_tick(push_tick.clone());
            let _ = black_box(tick_sub_lossless.try_recv());
        },
    );

    // 7. OrderManager idempotency lookup & submission (in-memory backend)
    let backend = DummyBackend::default();
    let mut mgr = OrderManager::new(998877);
    let mut order_idx = 0u64;
    bench("order_manager_submit_order (single-owner)", 20_000, || {
        order_idx += 1;
        let id = format!("b-{}", order_idx);
        let r = OrderRequest::buy("EURUSD", 0.1).client_order_id(&id);
        let _ = black_box(mgr.submit_order(&backend, r));
    });

    // 6. Reconciliation snapshot evaluation (50 positions + 50 deals matching)
    let mut recon_mgr = OrderManager::new(998877);
    let mut snap_positions = Vec::new();
    let mut snap_deals = Vec::new();
    for i in 0..50 {
        let cid = format!("recon-ord-{}", i);
        let r = OrderRequest::buy("EURUSD", 0.1).client_order_id(&cid);
        let tracked = recon_mgr.submit_order(&backend, r).unwrap();

        let mut p = Position::from_raw(raw_pos);
        p.ticket = tracked.position_ticket;
        p.volume = 0.1;
        p.magic = 998877;
        p.symbol = "EURUSD".to_string();
        p.comment = format!("cid:{}", tracked.wire_id);
        snap_positions.push(p);

        let mut d = Deal::from_raw(raw_deal);
        d.ticket = tracked.deal_ticket;
        d.position_id = tracked.position_ticket;
        d.order = tracked.order_ticket;
        d.volume = 0.1;
        d.magic = 998877;
        d.symbol = "EURUSD".to_string();
        snap_deals.push(d);
    }
    let snapshot = BrokerSnapshot {
        positions: snap_positions,
        pending_orders: vec![],
        deals: Some(snap_deals),
    };
    let now = chrono::Utc::now().timestamp();
    bench("reconcile_snapshot (50 orders evaluated)", 500, || {
        let _ = black_box(recon_mgr.reconcile_snapshot_at(snapshot.clone(), now));
    });

    // 7. Multi-threaded submission throughput under SharedOrderManager
    let shared_backend = Arc::new(DummyBackend::default());
    let shared_mgr = SharedOrderManager::new(OrderManager::new(998877));
    let num_threads = 4;
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
                let id = format!("th-{}-{}", t_id, i);
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
        "shared_order_manager (4 threads concurrent)", per_op_ns, ops_per_sec
    );

    println!("\n=== Benchmark Completed Successfully ===\n");
}

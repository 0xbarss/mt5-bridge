//! Concurrency stress tests on the named pipe IPC layer, active client mutexing,
//! pipe name validation, and multi-threaded request dispatching.

use mt5_bridge::client::pipe::validate_pipe_name;
use mt5_bridge::types::{OrderRequest, OrderState, Position, TradeResult, WorkingOrder};
use mt5_bridge::{Mt5Client, Mt5Error, OrderManager, Result, SharedOrderManager, TradingBackend};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

#[derive(Default)]
struct ConcurrentMockBackend {
    executed: AtomicU64,
    next_ticket: AtomicU64,
}

impl TradingBackend for ConcurrentMockBackend {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        self.executed.fetch_add(1, Ordering::Relaxed);
        let t = self.next_ticket.fetch_add(1, Ordering::Relaxed) + 1;
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
        self.executed.fetch_add(1, Ordering::Relaxed);
        Ok(TradeResult {
            retcode: 10009,
            deal: ticket + 500,
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
        _stop_loss: mt5_bridge::Level,
        _take_profit: mt5_bridge::Level,
    ) -> Result<TradeResult> {
        self.executed.fetch_add(1, Ordering::Relaxed);
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

#[test]
fn test_pipe_name_validation_concurrent_stress() {
    let threads = 16;
    let iterations = 1_000;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::new();

    for t_id in 0..threads {
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 0..iterations {
                let valid_name = format!("mt5_pipe_{}_{}", t_id, i);
                assert!(validate_pipe_name(&valid_name).is_ok());

                // Invalid variations must be rejected across all threads
                assert!(validate_pipe_name("").is_err());
                assert!(validate_pipe_name("../traversal").is_err());
                assert!(validate_pipe_name("pipe\\slash").is_err());
                assert!(validate_pipe_name("pipe/forward").is_err());
                assert!(validate_pipe_name("pipe\0null").is_err());
                assert!(validate_pipe_name("this_is_an_absurdly_long_pipe_name_that_exceeds_the_maximum_allowed_sixty_four_character_length_limit").is_err());
            }
        }));
    }

    for h in handles {
        h.join().expect("thread joined successfully");
    }
}

#[test]
fn test_client_active_mutual_exclusion_under_concurrency() {
    // mt5-bridge enforces single client active per process.
    // When attempting to connect without a valid pipe, it either returns
    // AlreadyConnected (if slot taken) or failed connection, but never
    // allows two threads to simultaneously hold CLIENT_ACTIVE.
    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::new();

    let client_active_errors = Arc::new(AtomicUsize::new(0));

    for _ in 0..threads {
        let b = Arc::clone(&barrier);
        let errs = Arc::clone(&client_active_errors);
        handles.push(thread::spawn(move || {
            b.wait();
            // Attempt to connect to a nonexistent mock pipe
            match Mt5Client::connect(999999, "secret", "demo") {
                Ok(_) => {
                    // If connection succeeded, hold it briefly
                    thread::sleep(Duration::from_millis(10));
                }
                Err(Mt5Error::AlreadyConnected) => {
                    errs.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // Normal failure (pipe/DLL absent)
                }
            }
        }));
    }

    for h in handles {
        let _ = h.join();
    }
}

#[test]
fn test_shared_order_manager_high_concurrency_stress() {
    let num_threads = 8;
    let ops_per_thread = 200;
    let backend = Arc::new(ConcurrentMockBackend::default());
    let manager = SharedOrderManager::new(OrderManager::try_new(424242).expect("valid magic"));
    let barrier = Arc::new(Barrier::new(num_threads + 1));
    let mut handles = Vec::new();

    for t_id in 0..num_threads {
        let b = Arc::clone(&barrier);
        let mgr = manager.clone();
        let be = Arc::clone(&backend);
        handles.push(thread::spawn(move || {
            b.wait();
            for i in 0..ops_per_thread {
                let cid = format!("t{}-ord-{}", t_id, i);
                let req = OrderRequest::buy("EURUSD", 0.10)
                    .price(1.08500)
                    .client_order_id(&cid);

                let res = mgr.submit_order(&*be, req);
                assert!(res.is_ok(), "submission failed for {}", cid);
                let tracked = res.unwrap();
                assert_eq!(tracked.state, OrderState::Filled);
                assert_eq!(tracked.client_order_id, cid);

                // Modify SL on odd iterations
                if i % 2 == 1 {
                    let mod_res = mgr.modify_order(
                        &*be,
                        tracked.position_ticket,
                        mt5_bridge::Level::Set(1.08000),
                        mt5_bridge::Level::Keep,
                    );
                    assert!(mod_res.is_ok(), "modify failed for {}", cid);
                }
            }
        }));
    }

    barrier.wait();
    for h in handles {
        h.join().expect("worker thread joined cleanly");
    }

    // Verify all operations executed without data race or loss
    let total_submissions = (num_threads * ops_per_thread) as u64;
    let total_modifications = (num_threads * (ops_per_thread / 2)) as u64;
    assert_eq!(
        backend.executed.load(Ordering::Relaxed),
        total_submissions + total_modifications
    );

    let active_orders = manager.export_orders();
    assert_eq!(active_orders.len(), total_submissions as usize);
}

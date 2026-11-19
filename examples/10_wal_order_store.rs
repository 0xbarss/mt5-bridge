use mt5_bridge::journal::OrderStore;
use mt5_bridge::types::{OrderRequest, OrderState, TrackedOrder};
use mt5_bridge::wal::{WalConfig, WalOrderStore};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MT5 Bridge: Durable Write-Ahead Log (WAL) Example ===");

    let temp_dir = std::env::temp_dir();
    let wal_path: PathBuf = temp_dir.join(format!(
        "mt5_wal_example_{}.wal",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    ));

    println!("Creating WAL at: {:?}", wal_path);

    // 1. Initialize WalOrderStore
    let config = WalConfig::default();
    let store = WalOrderStore::open_with_config(&wal_path, config)?;
    println!("✓ WAL initialized with CRC32 integrity verification.");

    // 2. Persist intent record before dispatching to broker
    let req = OrderRequest::buy("EURUSD", 0.50)
        .price(1.08500)
        .stop_loss(1.08000)
        .take_profit(1.09000);
    let mut order = TrackedOrder::new(&req, "strat-101-alpha");

    println!(
        "Persisting Intent for client_order_id='{}'...",
        order.client_order_id
    );
    store.record_intent(&order)?;
    println!("✓ Intent journaled to WAL (fsync completed).");

    // 3. Persist update record after broker responds
    order.state = OrderState::Filled;
    order.position_ticket = 88776655;
    order.deal_ticket = 11223344;
    order.order_ticket = 99001122;
    order.filled_volume = 0.50;
    order.average_price = 1.08502;

    println!(
        "Persisting Update: filled position_ticket={} deal_ticket={}...",
        order.position_ticket, order.deal_ticket
    );
    store.record_update(&order)?;
    println!("✓ Order execution update journaled.");

    // Flush and drop first handle to simulate shutdown
    store.flush()?;
    drop(store);
    println!("Closed active WAL store handle.");

    // 4. Recovery: Reopen store from disk
    println!("\nSimulating startup crash recovery...");
    let reopen_store = WalOrderStore::open(&wal_path)?;
    let recovered_orders = reopen_store.load()?;

    println!("✓ Recovered {} order(s) from disk:", recovered_orders.len());
    for rec_order in &recovered_orders {
        println!(
            "  - CID: {} | Symbol: {} | State: {:?} | Pos: {} | Deal: {} | Fill Price: {:.5}",
            rec_order.client_order_id,
            rec_order.symbol,
            rec_order.state,
            rec_order.position_ticket,
            rec_order.deal_ticket,
            rec_order.average_price
        );
    }
    assert_eq!(recovered_orders.len(), 1);
    assert_eq!(recovered_orders[0].state, OrderState::Filled);
    assert_eq!(recovered_orders[0].client_order_id, "strat-101-alpha");

    // 5. Compaction: Prune settled orders
    println!("\nCompacting WAL (pruning active vs settled orders)...");
    reopen_store.prune(&[order.client_order_id.clone()])?;
    println!("✓ Compaction snapshot written cleanly.");

    // Cleanup
    if wal_path.exists() {
        let _ = std::fs::remove_file(&wal_path);
        println!("Cleaned up example WAL file.");
    }

    println!("\n=== WAL demonstration completed successfully! ===");
    Ok(())
}

//! Integration tests for `WalOrderStore` crash recovery, compaction, CRC verification,
//! torn-tail recovery, position state journaling, and JSON migration.

use mt5_bridge::journal::OrderStore;
use mt5_bridge::position_book::{PositionRef, PositionState};
use mt5_bridge::types::{OrderRequest, OrderState, OrderType, TrackedOrder};
use mt5_bridge::wal::{migrate_json_to_wal, wal_to_json, WalConfig, WalOrderStore};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

fn temp_wal_path(name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mt5_test_{}_{}.wal", name, now))
}

#[test]
fn test_wal_full_lifecycle_and_reopen() {
    let path = temp_wal_path("lifecycle");

    // Phase 1: Create store, persist intents and updates
    {
        let store =
            WalOrderStore::open_with_config(&path, WalConfig::default()).expect("create wal store");
        assert!(store.is_incremental());

        let req1 = OrderRequest::buy("EURUSD", 0.50).price(1.08500);
        let mut ord1 = TrackedOrder::new(&req1, "cid-life-1");
        store.record_intent(&ord1).expect("record intent 1");

        let req2 = OrderRequest::sell("GBPUSD", 1.00).price(1.27000);
        let mut ord2 = TrackedOrder::new(&req2, "cid-life-2");
        store.record_intent(&ord2).expect("record intent 2");

        // Update ord1 to Filled
        ord1.state = OrderState::Filled;
        ord1.position_ticket = 1111;
        ord1.deal_ticket = 2222;
        ord1.filled_volume = 0.50;
        ord1.average_price = 1.08505;
        store.record_update(&ord1).expect("record update 1");

        // Update ord2 to Rejected
        ord2.state = OrderState::Rejected;
        ord2.error_message = Some("Margin exceeded".to_string());
        store.record_update(&ord2).expect("record update 2");

        store.flush().expect("flush store");
    }

    // Phase 2: Reopen store from disk and verify recovery
    {
        let reopen = WalOrderStore::open(&path).expect("reopen wal store");
        let orders = reopen.load().expect("load orders");

        assert_eq!(orders.len(), 2);

        let ord1 = orders
            .iter()
            .find(|o| o.client_order_id == "cid-life-1")
            .expect("ord1 present");
        assert_eq!(ord1.state, OrderState::Filled);
        assert_eq!(ord1.position_ticket, 1111);
        assert_eq!(ord1.deal_ticket, 2222);
        assert_eq!(ord1.filled_volume, 0.50);
        assert_eq!(ord1.average_price, 1.08505);

        let ord2 = orders
            .iter()
            .find(|o| o.client_order_id == "cid-life-2")
            .expect("ord2 present");
        assert_eq!(ord2.state, OrderState::Rejected);
        assert_eq!(ord2.error_message.as_deref(), Some("Margin exceeded"));
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_wal_torn_tail_crash_recovery() {
    let path = temp_wal_path("torn_tail");

    // Phase 1: Write two valid orders
    {
        let store = WalOrderStore::open(&path).expect("create wal store");
        let req1 = OrderRequest::buy("EURUSD", 0.10);
        let ord1 = TrackedOrder::new(&req1, "cid-good-1");
        store.record_intent(&ord1).expect("record intent 1");

        let req2 = OrderRequest::buy("EURUSD", 0.20);
        let ord2 = TrackedOrder::new(&req2, "cid-good-2");
        store.record_intent(&ord2).expect("record intent 2");
        store.flush().expect("flush");
    }

    // Phase 2: Corrupt file by appending partial / torn bytes (simulating sudden power loss during write)
    {
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open for corrupting");
        file.write_all(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03])
            .expect("append torn tail bytes");
        file.sync_all().expect("sync");
    }

    // Phase 3: Reopen - store must detect torn tail, truncate to last valid frame, and load valid orders
    {
        let store = WalOrderStore::open(&path).expect("reopen should recover from torn tail");
        let orders = store.load().expect("load recovered orders");
        assert_eq!(orders.len(), 2);
        assert!(orders.iter().any(|o| o.client_order_id == "cid-good-1"));
        assert!(orders.iter().any(|o| o.client_order_id == "cid-good-2"));

        // Store must still be writable after recovery
        let req3 = OrderRequest::buy("EURUSD", 0.30);
        let ord3 = TrackedOrder::new(&req3, "cid-after-recovery");
        store
            .record_intent(&ord3)
            .expect("record intent after recovery");
        store.flush().expect("flush");
    }

    // Phase 4: Verify 3 orders now present
    {
        let store = WalOrderStore::open(&path).expect("reopen 2");
        let orders = store.load().expect("load orders 2");
        assert_eq!(orders.len(), 3);
        assert!(orders
            .iter()
            .any(|o| o.client_order_id == "cid-after-recovery"));
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_wal_snapshot_compaction_pruning() {
    let path = temp_wal_path("prune");

    // Phase 1: Write 50 intent and update records
    {
        let store = WalOrderStore::open(&path).expect("create wal store");
        for i in 0..50 {
            let req = OrderRequest::buy("EURUSD", 0.10);
            let mut ord = TrackedOrder::new(&req, format!("cid-bulk-{}", i));
            store.record_intent(&ord).expect("record intent");

            ord.state = OrderState::Filled;
            ord.position_ticket = 1000 + i;
            store.record_update(&ord).expect("record update");
        }
        store.flush().expect("flush");

        let orders_before = store.load().expect("load 50");
        assert_eq!(orders_before.len(), 50);

        // Prune keeping only active orders 48 and 49
        let keep = vec!["cid-bulk-48".to_string(), "cid-bulk-49".to_string()];
        store.prune(&keep).expect("prune store");
    }

    // Phase 2: Reopen and verify compaction kept only active orders
    {
        let store = WalOrderStore::open(&path).expect("reopen pruned");
        let orders_after = store.load().expect("load pruned");
        assert_eq!(orders_after.len(), 2);
        assert!(orders_after
            .iter()
            .any(|o| o.client_order_id == "cid-bulk-48"));
        assert!(orders_after
            .iter()
            .any(|o| o.client_order_id == "cid-bulk-49"));
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_wal_middle_corruption_aborts_loading() {
    let path = temp_wal_path("middle_corrupt");

    // Write 3 records
    {
        let store = WalOrderStore::open(&path).expect("create wal store");
        for i in 0..3 {
            let req = OrderRequest::buy("EURUSD", 0.10);
            let ord = TrackedOrder::new(&req, format!("cid-corrupt-{}", i));
            store.record_intent(&ord).expect("record intent");
        }
        store.flush().expect("flush");
    }

    // Corrupt an intermediate frame byte
    {
        let mut data = std::fs::read(&path).expect("read wal");
        // Flip bits in the middle of frame 1 payload
        let mid = data.len() / 2;
        data[mid] ^= 0xFF;
        std::fs::write(&path, data).expect("write corrupted wal");
    }

    // Opening corrupted store should return an error due to CRC mismatch
    let result = WalOrderStore::open(&path);
    assert!(
        result.is_err(),
        "Store open must fail on corrupted middle frame"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_wal_position_state_persistence() {
    let path = temp_wal_path("positions");

    let pos1 = PositionRef::new(9001, "EURUSD", OrderType::Buy, 0.50, 1.0850);
    let pos2 = PositionRef::new(9002, "GBPUSD", OrderType::Sell, 1.00, 1.2700);

    {
        let store = WalOrderStore::open(&path).expect("create wal store");
        store
            .record_position(&pos1, PositionState::Open)
            .expect("record pos 1");
        store
            .record_position(&pos2, PositionState::Open)
            .expect("record pos 2");
        store.flush().expect("flush");
    }

    {
        let store = WalOrderStore::open(&path).expect("reopen wal store");
        let pos_map = store.load_position_states().expect("load pos states");
        assert_eq!(pos_map.len(), 2);
        assert_eq!(pos_map[&9001].0.symbol, "EURUSD");
        assert_eq!(pos_map[&9001].0.volume, 0.50);
        assert_eq!(pos_map[&9001].1, PositionState::Open);
        assert_eq!(pos_map[&9002].0.symbol, "GBPUSD");
        assert_eq!(pos_map[&9002].1, PositionState::Open);
        assert_eq!(store.max_position_id().expect("max id"), Some(9002));
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_wal_and_json_migration() {
    let wal_path = temp_wal_path("mig_wal");
    let json_path = temp_wal_path("mig_json");

    // 1. Create WAL with orders
    {
        let store = WalOrderStore::open(&wal_path).expect("create wal");
        let req = OrderRequest::buy("EURUSD", 0.75);
        let mut ord = TrackedOrder::new(&req, "cid-mig-1");
        ord.state = OrderState::Filled;
        ord.position_ticket = 7788;
        store.record_intent(&ord).expect("intent");
        store.record_update(&ord).expect("update");
        store.flush().expect("flush");
    }

    // 2. Export WAL to JSON
    wal_to_json(&wal_path, &json_path).expect("wal to json");
    assert!(json_path.exists());

    // 3. Migrate JSON to a new WAL
    let new_wal_path = temp_wal_path("mig_wal2");
    migrate_json_to_wal(&json_path, &new_wal_path).expect("json to wal");

    // 4. Verify migrated WAL has identical order
    {
        let store = WalOrderStore::open(&new_wal_path).expect("open migrated wal");
        let orders = store.load().expect("load migrated orders");
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].client_order_id, "cid-mig-1");
        assert_eq!(orders[0].state, OrderState::Filled);
        assert_eq!(orders[0].position_ticket, 7788);
        assert_eq!(orders[0].requested_volume, 0.75);
    }

    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&json_path);
    let _ = std::fs::remove_file(&new_wal_path);
}

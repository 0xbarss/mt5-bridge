# Operational Runbook — `mt5-bridge`

This runbook guides operators through diagnosing and resolving critical runtime incidents when operating `mt5-bridge` with MetaTrader 5 terminals.

---

## 1. Incident: Orders in `OrderState::Unknown`

### Symptoms
- Strategy logs report `OrderState::Unknown` for one or more orders.
- Subsequent order submissions fail with `Mt5Error::UnresolvedUnknownOrders` (under default `UnknownBlockScope::Strategy`).
- Terminal connectivity was briefly lost, timed out, or restarted during an order submission.

### Root Cause
An order packet was transmitted to the broker/EA, but the confirmation response did not return before pipe disconnect or timeout. Per the core design invariant (**transport failure ≠ order rejection**), the bridge does not assume the order failed, preventing catastrophic double exposure.

### Automated Resolution
1. The `OrderManager` triggers multi-pass reconciliation against MT5 live positions (`CMD_POSITIONS_GET`), working orders (`CMD_ORDERS_GET`), and deal history (`CMD_DEALS_GET`).
2. Per `SafetyPolicy::default()`:
   - Definite rejection requires **3 consecutive absent passes** spanning at least **30 seconds**.
   - MT5 deal history must be available and evaluated.
3. If the order was actually filled or placed, reconciliation automatically matches the position or order ticket via comment wire token (`w:<token>`) or deal ticket, transitioning the order to `Filled` or `Accepted`.
4. If the order was definitively absent across all passes, it transitions safely to `Rejected`.

### Operator Intervention Checklist
If an order remains `Unknown` beyond the grace period:
1. Open the MT5 Terminal GUI and inspect the **Trade** and **History** tabs.
2. Search for the strategy magic number or the order comment containing `w:<token>`.
3. If an open position or deal exists:
   - Run a manual reconciliation pass: `manager.reconcile(&client)`.
4. If no execution occurred and you have verified the broker rejected the order:
   - In code or admin CLI, invoke `manager.acknowledge_rejection(client_order_id)` or call `reconcile_until_settled(&client, timeout)`.
5. Once resolved, the submission gate clears and new orders are accepted.

---

## 2. Incident: Mismatched Orders (`OrderState::Mismatched`)

### Symptoms
- An order matches an active position or deal ticket, but enters `OrderState::Mismatched`.
- Reconciliation report shows items under `report.mismatches`.

### Root Cause
Attribute mismatch between expected client intent and actual broker execution. Possible categories:
- `VolumeMismatch`: Executed volume does not match requested volume (partial fill without remainder, or broker lot adjustment).
- `PriceMismatch`: Fill price deviated beyond acceptable tolerance.
- `ProtectionMismatch`: Broker adjusted or stripped Stop Loss or Take Profit.
- `DirectionMismatch` / `SymbolMismatch`: Critical execution anomaly.

### Operator Intervention Checklist
1. Inspect the mismatch details via `report.mismatches`.
2. Check broker slippage or execution rules in the MT5 **Journal** and **Experts** tabs.
3. If the discrepancy is understood and accepted as valid broker execution:
   - Acknowledge the mismatch: `manager.acknowledge_mismatch(client_order_id)`.
   - The order transitions to `Filled` or `PartiallyFilled` based on executed volume.

---

## 3. Incident: Write-Ahead Log (WAL) Recovery & Maintenance

### Binary WAL Design
- `mt5-bridge` stores durable execution intent in a binary WAL (`orders.wal`) using format magic `MT5WAL01`.
- Every frame includes a CRC32 checksum, record length, and monotonic sequence number.
- `OrderManager` writes and fsyncs a `Submitting` intent **before** transmitting to the broker.

### Torn-Tail Recovery
- If the machine or process crashes mid-write, the final uncompleted frame at the tail of `orders.wal` will fail CRC32 validation.
- On startup, `WalOrderStore::open()` detects torn tails at EOF and safely truncates the damaged trailing bytes, restoring all preceding records intact.
- If corruption occurs mid-file (not at EOF), startup fails with a hard error to prevent running with corrupted historical state.

### Snapshots & Compaction
- Over time, append-only logs accumulate historical transitions.
- Periodic compaction creates an atomic snapshot `orders.snap` (`orders.snap.tmp` -> atomic rename) and creates a fresh `orders.wal`.
- To trigger compaction programmatically:
  ```rust
  manager.compact_store()?;
  ```

### Migration Between Formats
- To convert a legacy JSON journal to binary WAL:
  ```rust
  mt5_bridge::migrate_json_to_wal("orders.json", "orders.wal")?;
  ```
- To dump a binary WAL to human-readable JSON for audit or inspection:
  ```rust
  mt5_bridge::wal_to_json("orders.wal", "orders_dump.json")?;
  ```

---

## 4. Emergency Procedures: Terminal Restart & Headless Takeover

### MT5 Terminal Restart
1. When MT5 restarts, the Expert Advisor `mt5_bridge.mq5` restarts its named pipe server on chart initialization (`OnInit`).
2. The client DLL detects pipe disconnection (`MT5_ERR_PIPE_DISCONNECTED`) and tears down the invalid handle.
3. Reconnect the client using `Mt5Client::connect(...)`.
4. Run `manager.reconcile(&client)` immediately to restore state before resuming order submission.

### Headless Takeover
- When deploying in headless environments (Linux / Wine or Windows Server), see scripts in `mql5/autostart/`.
- The EA checks `InpRequireSecret` and `InpPipeSDDL` on startup. Ensure the `MT5_PIPE_SECRET` environment variable matches the terminal configuration.

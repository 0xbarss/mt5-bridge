# Protocol & API Migration Guide — `mt5-bridge`

This document details protocol evolution across versions and provides migration guidance for integrators updating across `mt5-bridge` releases.

---

## 1. Protocol Version Evolution

| Version | Protocol ID | Highlights & Breaking Wire Changes |
| :---: | :---: | :--- |
| **v4** | `4` | Introduced historical deal history querying (`CMD_DEALS_GET`, `Mt5Deal` 152 bytes). |
| **v5** | `5` | Added push event streaming (`OnTick`, `OnTradeTransaction`, `OnBookEvent`). Added wire ID tokenization in order comments (`w:<token>`). |
| **v6** | `6` | Expanded `PacketHdr` from 12 bytes to 16 bytes with `request_id`. Added `event_seq` to `Mt5TickEvent` (88 B), `Mt5TradeEvent` (176 B), `Mt5BookEvent` (72 B). |
| **v7** | `7` | Expanded `Mt5SymInfo` from 60 bytes to 76 bytes (added `bid` and `ask`). Added closed bar streaming (`CMD_SUBSCRIBE_BARS`, `CMD_UNSUBSCRIBE_BARS`, `Mt5BarEvent` 96 bytes). |
| **v8** | `8` *(Planned)* | Planned timestamp unification across book events (epoch ms) and per-stream sequence gap detection. |

---

## 2. Migrating to Protocol v7 / Crate 0.3.0

### Wire Protocol Updates
1. **`PacketHdr` is 16 bytes:**
   - Previous versions used a 12-byte header. Protocol v6+ requires a 16-byte header with a 4-byte `request_id` field.
2. **`Mt5SymInfo` is 76 bytes:**
   - Two `f64` fields (`bid` and `ask`) were appended to `Mt5SymInfo`. Clients reading raw bytes must account for the 76-byte layout.
3. **Event Struct Sizes:**
   - `Mt5TickEvent`: 88 bytes.
   - `Mt5TradeEvent`: 176 bytes.
   - `Mt5BookEvent`: 72 bytes.
   - `Mt5BarEvent`: 96 bytes.

### Rust API Renames & Adjustments
1. **Streaming Method Names:**
   - `subscribe_trades()` → [`Mt5Client::subscribe_trade()`](file:///mnt/data6/MyFiles/Projects/mt5-bridge/src/client/mod.rs)
   - `unsubscribe_trades()` → [`Mt5Client::unsubscribe_trade()`](file:///mnt/data6/MyFiles/Projects/mt5-bridge/src/client/mod.rs)
   - `subscribe_depth()` → [`Mt5Client::subscribe_book()`](file:///mnt/data6/MyFiles/Projects/mt5-bridge/src/client/mod.rs)
   - `unsubscribe_depth()` → [`Mt5Client::unsubscribe_book()`](file:///mnt/data6/MyFiles/Projects/mt5-bridge/src/client/mod.rs)
   - Added: [`Mt5Client::subscribe_bars()`](file:///mnt/data6/MyFiles/Projects/mt5-bridge/src/client/mod.rs) / [`unsubscribe_bars()`](file:///mnt/data6/MyFiles/Projects/mt5-bridge/src/client/mod.rs)
2. **Channel Types:**
   - `subscribe_book` returns `BookSubscription` (with `recv_checked()` sequence gap detection and drop pruning).
   - `subscribe_trade` returns `tokio::sync::broadcast::Receiver<TradeEvent>`.
   - `subscribe_ticks` returns `TickSubscription` with configurable mode (`StreamMode::Latest` or `StreamMode::Lossless`).
3. **Magic Number Default Invariant:**
   - Introduced `EA_DEFAULT_MAGIC` (`20240101`) matching the MQL5 EA `InpMagicNumber` parameter. Deprecated `DEFAULT_MAGIC` (`424242`).
   - The MQL5 EA parameter `InpEnforceMagicNumber` defaults to `true`.
   - `OrderManager::submit_order` populates `req.magic` with `strategy_magic` when `req.magic` is `None` (never silently defaulting to legacy `DEFAULT_MAGIC`).
   - When submitting orders, ensure the client magic number matches the EA's `InpMagicNumber` (or set `InpEnforceMagicNumber = false` in the EA parameters if multiple strategies share the terminal).
4. **Session Supervision:**
   - `Mt5Session` manages connection lifecycle states (`Starting`, `Connecting`, `Authenticating`, `Syncing`, `Reconciling`, `Ready`, `Degraded`, `Stopped`).

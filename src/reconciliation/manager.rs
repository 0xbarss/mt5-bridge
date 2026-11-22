use crate::backend::TradingBackend;
use crate::error::{Mt5Error, Result};
use crate::journal::OrderStore;
use crate::reconciliation::classification::classify_send_error;
use crate::reconciliation::state::{
    BrokerSnapshot, LifecycleState, ReconciliationReport, SafetyPolicy, UnknownBlockScope,
    DEAL_WINDOW_SLACK_SECS,
};
use crate::types::{
    comment_matches_client_order_id, parse_wire_id, wire_id, Deal, Level, OrderRequest, OrderState,
    OrderType, Position, RejectionEvidence, TrackedOrder, TradeEvent, TradeResult, WorkingOrder,
    MAX_CLIENT_ORDER_ID_BYTES,
};
use chrono::Utc;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

static ID_SEQ: AtomicU64 = AtomicU64::new(0);

/// Strongly-typed non-zero strategy magic number for order identification and isolation.
///
/// In MetaTrader 5 and this bridge, a non-zero magic number uniquely identifies
/// orders and positions owned by a specific trading strategy or expert advisor instance.
/// A magic number of zero indicates unmanaged manual trades or an unconfigured strategy,
/// which cannot provide isolation guarantees.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct StrategyMagic(NonZeroU64);

impl StrategyMagic {
    /// Create a new `StrategyMagic` after validating that `value` is non-zero.
    pub fn new(value: u64) -> Result<Self> {
        NonZeroU64::new(value).map(Self).ok_or_else(|| {
            Mt5Error::InvalidRequest("strategy magic number must be non-zero".to_string())
        })
    }

    /// Construct a `StrategyMagic` from a known non-zero value at compile time or in const contexts.
    pub const fn const_new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Construct a `StrategyMagic` directly from a [`NonZeroU64`].
    pub const fn from_non_zero(value: NonZeroU64) -> Self {
        Self(value)
    }

    /// Return the raw `u64` magic number value.
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Return the underlying [`NonZeroU64`].
    pub const fn as_non_zero(self) -> NonZeroU64 {
        self.0
    }
}

impl std::fmt::Display for StrategyMagic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<StrategyMagic> for u64 {
    fn from(magic: StrategyMagic) -> Self {
        magic.get()
    }
}

impl From<StrategyMagic> for NonZeroU64 {
    fn from(magic: StrategyMagic) -> Self {
        magic.0
    }
}

impl From<NonZeroU64> for StrategyMagic {
    fn from(value: NonZeroU64) -> Self {
        Self(value)
    }
}

impl TryFrom<u64> for StrategyMagic {
    type Error = Mt5Error;

    fn try_from(value: u64) -> Result<Self> {
        Self::new(value)
    }
}

impl PartialEq<u64> for StrategyMagic {
    fn eq(&self, other: &u64) -> bool {
        self.0.get() == *other
    }
}

impl PartialEq<StrategyMagic> for u64 {
    fn eq(&self, other: &StrategyMagic) -> bool {
        *self == other.0.get()
    }
}

/// High-level execution and order lifecycle manager.
///
/// Wraps a [`TradingBackend`] (normally [`Mt5Client`](crate::Mt5Client)) with:
/// - Order idempotency: ensures repeated submits with the same `client_order_id` do not create duplicate exposure.
/// - Explicit order state machine: tracks volume and states (`Submitting`, `Filled`, `Partial`, `Unknown`).
/// - Post-failure reconciliation: queries live broker state and resolves ambiguous execution outcomes.
/// - Strategy ownership: enforces strategy magic number on all close, modify, and query actions.
/// - An Unknown-execution submission gate and write-ahead journaling (see the module docs).
///
/// See the [module-level concurrency contract](crate::reconciliation#concurrency-contract): this type is
/// single-owner; use [`SharedOrderManager`](crate::reconciliation::SharedOrderManager) to share it between threads.
pub struct OrderManager {
    strategy_magic: StrategyMagic,
    lifecycle: LifecycleState,
    orders: BTreeMap<String, TrackedOrder>,
    /// wire token → full `client_order_id`; detects hash collisions between distinct IDs.
    wire_index: HashMap<String, String>,
    policy: SafetyPolicy,
    store: Option<Box<dyn OrderStore>>,
}

impl OrderManager {
    /// Create a new `OrderManager` bound to a validated [`StrategyMagic`] number, with no
    /// durable journal (state lives in memory only — see [`with_store`](Self::with_store)).
    pub fn new(strategy_magic: StrategyMagic) -> Self {
        Self {
            strategy_magic,
            lifecycle: LifecycleState::Starting,
            orders: BTreeMap::new(),
            wire_index: HashMap::new(),
            policy: SafetyPolicy::default(),
            store: None,
        }
    }

    /// Safely create a new `OrderManager` validating that strategy magic is non-zero.
    pub fn try_new(strategy_magic: u64) -> Result<Self> {
        let magic = StrategyMagic::new(strategy_magic)?;
        Ok(Self::new(magic))
    }

    /// Create a manager with an explicit [`SafetyPolicy`] and validated [`StrategyMagic`].
    pub fn with_policy(strategy_magic: StrategyMagic, policy: SafetyPolicy) -> Self {
        let mut m = Self::new(strategy_magic);
        m.policy = policy;
        m
    }

    /// Safely create a manager with an explicit [`SafetyPolicy`], validating that strategy magic is non-zero.
    pub fn try_with_policy(strategy_magic: u64, policy: SafetyPolicy) -> Result<Self> {
        let magic = StrategyMagic::new(strategy_magic)?;
        Ok(Self::with_policy(magic, policy))
    }

    /// Create a manager backed by a durable [`OrderStore`], restoring any state it holds.
    ///
    /// From here on every state change is journaled, and — critically — a `Submitting`
    /// record is written and made durable **before** an order is transmitted. If the process
    /// dies mid-request, the next start finds that record, treats it as `Unknown`, and
    /// reconciliation resolves it against the broker. Orders restored in `Submitting` state
    /// are converted to `Unknown` for exactly that reason.
    ///
    /// Fails (rather than starting empty) if the store exists but cannot be read, or if
    /// `strategy_magic` is zero.
    pub fn with_store(strategy_magic: u64, store: Box<dyn OrderStore>) -> Result<Self> {
        let magic = StrategyMagic::new(strategy_magic)?;
        Self::with_store_magic(magic, store)
    }

    /// Create a manager backed by a durable [`OrderStore`] with a validated [`StrategyMagic`],
    /// restoring any state it holds.
    ///
    /// Fails (rather than starting empty) if the store exists but cannot be read.
    pub fn with_store_magic(
        strategy_magic: StrategyMagic,
        store: Box<dyn OrderStore>,
    ) -> Result<Self> {
        let restored = store.load()?;
        let mut m = Self::new(strategy_magic);
        m.restore_orders(restored);
        m.store = Some(store);
        Ok(m)
    }

    /// Current safety policy.
    pub fn policy(&self) -> &SafetyPolicy {
        &self.policy
    }

    /// Replace the safety policy.
    pub fn set_policy(&mut self, policy: SafetyPolicy) {
        self.policy = policy;
    }

    /// Current operational lifecycle state.
    pub fn lifecycle(&self) -> LifecycleState {
        self.lifecycle
    }

    /// Set current operational lifecycle state.
    pub fn set_lifecycle(&mut self, state: LifecycleState) {
        self.lifecycle = state;
    }

    /// Strategy magic number owned by this manager as a primitive `u64`.
    pub fn strategy_magic(&self) -> u64 {
        self.strategy_magic.get()
    }

    /// Validated [`StrategyMagic`] owned by this manager.
    pub fn magic(&self) -> StrategyMagic {
        self.strategy_magic
    }

    /// Retrieve all tracked orders (ordered by `client_order_id`).
    pub fn tracked_orders(&self) -> Vec<&TrackedOrder> {
        self.orders.values().collect()
    }

    /// Find a tracked order by its unique client order ID.
    pub fn get_order(&self, client_order_id: &str) -> Option<&TrackedOrder> {
        self.orders.get(client_order_id)
    }

    /// Orders whose broker-side outcome is still undetermined (`Unknown` / `Submitting`).
    pub fn unresolved_orders(&self) -> Vec<&TrackedOrder> {
        self.orders.values().filter(|o| o.is_unresolved()).collect()
    }

    /// Generate a unique client order ID.
    ///
    /// Combines a nanosecond timestamp, the process ID and a process-wide sequence number, so
    /// two calls can never return the same value — even on platforms with coarse clocks or
    /// from concurrent threads. (A timestamp alone can repeat, and a repeated ID would be
    /// mistaken for an idempotent replay, silently dropping the second order.)
    pub fn generate_client_order_id() -> String {
        let ts = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
        format!("ord-{:x}-{:x}-{:x}", ts, std::process::id(), seq)
    }

    /// Register a tracked order directly into the manager (e.g. during restart recovery from persistent storage).
    ///
    /// The state is taken as-is; use [`restore_orders`](Self::restore_orders) for recovery, which also
    /// treats a stale `Submitting` record as `Unknown`. This does not write to the journal.
    pub fn track_order(&mut self, mut order: TrackedOrder) {
        if order.wire_id.is_empty() {
            order.wire_id = wire_id(&order.client_order_id);
        }
        match self.wire_index.get(&order.wire_id) {
            Some(existing) if existing != &order.client_order_id => {
                error!(
                    wire_id = %order.wire_id,
                    existing = %existing,
                    incoming = %order.client_order_id,
                    "wire token collision between distinct client order IDs while tracking order"
                );
            }
            _ => {
                self.wire_index
                    .insert(order.wire_id.clone(), order.client_order_id.clone());
            }
        }
        self.orders.insert(order.client_order_id.clone(), order);
    }

    /// Restore multiple tracked orders into the manager during restart recovery.
    ///
    /// Any order found in `Submitting` state is converted to `Unknown`: a `Submitting`
    /// record that survived a restart means the process stopped somewhere between "about to
    /// transmit" and "outcome recorded", so the request may or may not have reached the broker.
    pub fn restore_orders(&mut self, orders: impl IntoIterator<Item = TrackedOrder>) {
        for mut order in orders {
            if order.state == OrderState::Submitting {
                order.mark_unknown(
                    "restored in Submitting state: process stopped during transmission; broker outcome unknown",
                );
            }
            self.track_order(order);
        }
    }

    /// Export all tracked orders for persistent storage or state serialization.
    ///
    /// This is an **in-memory snapshot primitive only** and gives no crash-safety by itself.
    /// For durability use [`with_store`](Self::with_store), which journals automatically and
    /// writes ahead of every transmission.
    pub fn export_orders(&self) -> Vec<TrackedOrder> {
        self.orders.values().cloned().collect()
    }

    /// Clone the tracked order book keyed by `client_order_id`.
    ///
    /// Used by [`SharedOrderManager`](crate::reconciliation::SharedOrderManager) to publish a
    /// lock-free read snapshot after every mutation.
    pub(crate) fn orders_snapshot(&self) -> BTreeMap<String, TrackedOrder> {
        self.orders.clone()
    }

    /// Flush the current state to the configured [`OrderStore`] (no-op without one).
    pub fn persist(&self) -> Result<()> {
        match &self.store {
            Some(store) => store.save(&self.export_orders()),
            None => Ok(()),
        }
    }

    /// Journal pre-send order intent.
    /// If the store is incremental, records only the intent with sync fsync;
    /// otherwise falls back to a full snapshot via `persist()`.
    pub fn persist_intent(&self, order: &TrackedOrder) -> Result<()> {
        match &self.store {
            Some(store) if store.is_incremental() => store.record_intent(order),
            Some(_) => self.persist(),
            None => Ok(()),
        }
    }

    /// Journal an order update.
    /// If the store is incremental, appends the update without full rewrite;
    /// otherwise falls back to a full snapshot via `persist()`.
    pub fn persist_update(&self, order: &TrackedOrder) -> Result<()> {
        match &self.store {
            Some(store) if store.is_incremental() => store.record_update(order),
            Some(_) => self.persist(),
            None => Ok(()),
        }
    }

    /// Prunes terminal orders (Cancelled, Rejected, or Filled) older than `max_age_seconds`
    /// from in-memory tracking and backing store.
    ///
    /// Returns the number of orders pruned.
    pub fn prune_settled_orders(&mut self, max_age_seconds: i64) -> usize {
        let now = Utc::now().timestamp();
        let mut pruned_cids = Vec::new();

        for (cid, order) in &self.orders {
            let is_terminal = matches!(
                order.state,
                OrderState::Cancelled | OrderState::Rejected | OrderState::Filled
            );
            if is_terminal && (now - order.updated_at) >= max_age_seconds {
                pruned_cids.push(cid.clone());
            }
        }

        if pruned_cids.is_empty() {
            return 0;
        }

        for cid in &pruned_cids {
            if let Some(order) = self.orders.remove(cid) {
                self.wire_index.remove(&order.wire_id);
            }
        }

        let retained_cids: Vec<String> = self.orders.keys().cloned().collect();
        if let Some(store) = &self.store {
            if store.is_incremental() {
                let _ = store.prune(&retained_cids);
            } else {
                let _ = store.save(&self.export_orders());
            }
        }

        pruned_cids.len()
    }

    /// Validates strategy magic ownership and checks the idempotency guard for an order request.
    ///
    /// Returns:
    /// - `Ok(Some(tracked))` if the order was already executed/tracked (idempotent replay).
    /// - `Ok(None)` if the order is new and must be transmitted to the broker.
    /// - `Err(Mt5Error::OwnershipMismatch)` if the order's magic number violates strategy ownership.
    /// - `Err(Mt5Error::ClientOrderIdConflict)` if the ID was already used for a *different*
    ///   request (symbol / type / volume differ), or its wire token collides with another ID.
    pub fn check_idempotency_and_magic(
        &self,
        req: &mut OrderRequest,
    ) -> Result<Option<TrackedOrder>> {
        if req.magic.is_none() || req.magic == Some(0) {
            req.magic = Some(self.strategy_magic.get());
        } else if req.magic != Some(self.strategy_magic.get()) {
            return Err(Mt5Error::OwnershipMismatch {
                ticket: 0,
                expected_magic: self.strategy_magic.get(),
                actual_magic: req.magic.unwrap_or(0),
            });
        }

        if let Some(cid) = &req.client_order_id {
            if let Some(existing) = self.orders.get(cid) {
                let same_intent = existing.symbol.eq_ignore_ascii_case(&req.symbol)
                    && existing.order_type == req.order_type
                    && (existing.requested_volume - req.volume).abs() <= 1e-8;
                if !same_intent {
                    return Err(Mt5Error::ClientOrderIdConflict {
                        client_order_id: cid.clone(),
                        reason: format!(
                            "already used for {:?} {} {} lots, not {:?} {} {} lots",
                            existing.order_type,
                            existing.symbol,
                            existing.requested_volume,
                            req.order_type,
                            req.symbol,
                            req.volume
                        ),
                    });
                }
                return Ok(Some(existing.clone()));
            }

            let token = wire_id(cid);
            if let Some(other) = self.wire_index.get(&token) {
                if other != cid {
                    return Err(Mt5Error::ClientOrderIdConflict {
                        client_order_id: cid.clone(),
                        reason: format!(
                            "its wire identity '{token}' collides with existing client_order_id '{other}'"
                        ),
                    });
                }
            }
        }

        Ok(None)
    }

    /// Orders that currently block a new submission for `symbol` under the active
    /// [`UnknownBlockScope`].
    fn blocking_orders(&self, symbol: &str) -> Vec<String> {
        let scope = self.policy.unknown_block_scope;
        self.orders
            .values()
            .filter(|o| o.is_unresolved())
            .filter(|o| match scope {
                UnknownBlockScope::Strategy => true,
                UnknownBlockScope::Symbol => o.symbol.eq_ignore_ascii_case(symbol),
                UnknownBlockScope::Disabled => false,
            })
            .map(|o| o.client_order_id.clone())
            .collect()
    }

    /// Submit a trading order with idempotency protection and lifecycle state tracking.
    ///
    /// Sequence (all under the caller's exclusive `&mut self` — see the concurrency contract):
    /// 1. Validate the request (invalid requests never touch state).
    /// 2. Idempotency & Magic: a repeated `client_order_id` returns the existing record and never
    ///    re-contacts the broker. If `req.magic` is `None` (or 0), it is assigned `strategy_magic`.
    ///    Any non-matching magic number is rejected with [`Mt5Error::OwnershipMismatch`].
    /// 3. Unknown gate: refuse while any order's outcome is unresolved
    ///    ([`Mt5Error::UnresolvedUnknownOrders`]).
    /// 4. Write-ahead: record `Submitting` and, if a store is configured, make it durable.
    ///    **If that fails the order is not transmitted.**
    /// 5. Transmit, then record the outcome (`Filled` / `Accepted` / `Rejected` / `Unknown`).
    pub fn submit_order<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
        mut req: OrderRequest,
    ) -> Result<TrackedOrder> {
        req.validate()?;
        let cid = req
            .client_order_id
            .clone()
            .unwrap_or_else(Self::generate_client_order_id);
        req.client_order_id = Some(cid.clone());

        if let Some(existing) = self.check_idempotency_and_magic(&mut req)? {
            warn!(
                client_order_id = %cid,
                state = ?existing.state,
                "Idempotency guard: order with client_order_id already exists; returning tracked state"
            );
            return Ok(existing);
        }

        let blockers = self.blocking_orders(&req.symbol);
        if !blockers.is_empty() {
            warn!(
                client_order_id = %cid,
                blockers = ?blockers,
                "Submission blocked: unresolved UNKNOWN order(s) must be reconciled first"
            );
            return Err(Mt5Error::UnresolvedUnknownOrders {
                symbol: if self.policy.unknown_block_scope == UnknownBlockScope::Symbol {
                    req.symbol.clone()
                } else {
                    String::new()
                },
                client_order_ids: blockers,
            });
        }

        // Write-ahead record.
        let mut tracked = TrackedOrder::new(&req, &cid);
        tracked.mark_submitting();
        self.track_order(tracked.clone());
        if let Err(e) = self.persist_intent(&tracked) {
            // Not durable ⇒ not sent. Roll the in-memory record back so a retry is possible.
            self.orders.remove(&cid);
            self.wire_index.remove(&tracked.wire_id);
            error!(client_order_id = %cid, error = %e, "Write-ahead journal failed; order NOT transmitted");
            return Err(e);
        }

        // Transmit to MT5 bridge
        let outcome = client.order_send(&req);
        let result = match outcome {
            Ok(trade_res) => {
                tracked.update_from_trade_result(&trade_res);
                info!(
                    client_order_id = %cid,
                    state = ?tracked.state,
                    order = tracked.order_ticket,
                    deal = tracked.deal_ticket,
                    price = tracked.average_price,
                    "Order executed and tracked successfully"
                );
                Ok(())
            }
            Err(e) => {
                let (new_state, evidence, desc) = classify_send_error(&e);
                match new_state {
                    OrderState::Unknown => {
                        tracked.mark_unknown(&desc);
                        self.lifecycle = LifecycleState::Degraded;
                        warn!(
                            client_order_id = %cid,
                            "Order execution outcome is UNKNOWN! Reconcile against broker before retrying."
                        );
                        Err(Mt5Error::UnknownExecutionState {
                            symbol: req.symbol.clone(),
                            client_order_id: Some(cid.clone()),
                            description: desc,
                        })
                    }
                    OrderState::Rejected => {
                        tracked.state = OrderState::Rejected;
                        tracked.rejection = evidence;
                        tracked.error_message = Some(desc);
                        tracked.updated_at = Utc::now().timestamp();
                        Err(e)
                    }
                    _ => Err(e),
                }
            }
        };

        self.track_order(tracked.clone());
        if let Err(e) = self.persist_update(&tracked) {
            // The write-ahead record (Submitting → Unknown on restore) still protects us.
            error!(client_order_id = %cid, error = %e, "Failed to journal order outcome; durable state is stale but safe");
        }

        result.map(|()| tracked)
    }

    /// Reconcile local tracked orders against live MT5 positions, working orders and deal history.
    ///
    /// Each pass counts as **one observation** for every unresolved order not found anywhere.
    /// An order is only declared unplaced (`Rejected`) after the [`SafetyPolicy`]'s required
    /// number of observations spanning its minimum duration — so call this repeatedly (or use
    /// [`reconcile_until_settled`](Self::reconcile_until_settled)) rather than trusting one
    /// snapshot. Orders that turn up are verified attribute-by-attribute (`Reconciled` or
    /// `Mismatched`).
    ///
    /// Deal history is fetched only when something is unresolved; if the backend cannot supply
    /// it, the pass proceeds without and — under the default policy — leaves absent orders
    /// `Unknown` rather than guessing.
    pub fn reconcile<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
    ) -> Result<ReconciliationReport> {
        self.lifecycle = LifecycleState::Reconciling;
        let now = Utc::now().timestamp();

        let snapshot = match self.fetch_snapshot(client, now) {
            Ok(s) => s,
            Err(e) => {
                self.lifecycle = LifecycleState::Degraded;
                return Err(e);
            }
        };
        let report = self.reconcile_snapshot_at(snapshot, now);
        if let Err(e) = self.persist() {
            self.lifecycle = LifecycleState::Degraded;
            error!(error = %e, "Failed to journal reconciliation results");
            return Err(e);
        }
        Ok(report)
    }

    fn fetch_snapshot<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        now: i64,
    ) -> Result<BrokerSnapshot> {
        let positions = client.positions_filtered(Some(self.strategy_magic.get()), None)?;
        let pending_orders =
            client.pending_orders_filtered(Some(self.strategy_magic.get()), None)?;

        let oldest_unresolved = self
            .orders
            .values()
            .filter(|o| o.is_unresolved())
            .map(|o| o.created_at)
            .min();
        let deals = match oldest_unresolved {
            None => None,
            Some(oldest) => {
                let from = oldest - DEAL_WINDOW_SLACK_SECS;
                let to = now + DEAL_WINDOW_SLACK_SECS;
                match client.deals_filtered(from, to, Some(self.strategy_magic.get()), None) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        warn!(error = %e, "Deal history unavailable for reconciliation; absent orders will stay Unknown");
                        None
                    }
                }
            }
        };
        Ok(BrokerSnapshot {
            positions,
            pending_orders,
            deals,
        })
    }

    /// Call [`reconcile`](Self::reconcile) every `interval` until no order is unresolved or
    /// `timeout` elapses, returning the last report.
    ///
    /// # Blocking
    ///
    /// This method is synchronous and blocks the calling thread via [`std::thread::sleep`].
    /// In asynchronous contexts (such as Tokio runtimes), calling this directly on a runtime
    /// worker thread will block the executor. Use [`reconcile_until_settled_async`](Self::reconcile_until_settled_async)
    /// or wrap the call in `tokio::task::spawn_blocking`.
    pub fn reconcile_until_settled<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
        interval: Duration,
        timeout: Duration,
    ) -> Result<ReconciliationReport> {
        let start = Instant::now();
        loop {
            let report = self.reconcile(client)?;
            let elapsed = start.elapsed();
            if report.is_settled() || elapsed >= timeout {
                return Ok(report);
            }
            std::thread::sleep(interval.min(timeout.saturating_sub(elapsed)));
        }
    }

    /// Asynchronously call [`reconcile`](Self::reconcile) every `interval` until no order is
    /// unresolved or `timeout` elapses, returning the last report without blocking executor threads.
    ///
    /// Yields cooperatively between reconciliation passes using [`tokio::time::sleep`].
    /// Note that each [`reconcile`](Self::reconcile) pass performs synchronous queries against
    /// the provided [`TradingBackend`]; if backend operations block, each pass can be dispatched
    /// via `tokio::task::spawn_blocking`.
    #[cfg(feature = "async")]
    pub async fn reconcile_until_settled_async<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
        interval: Duration,
        timeout: Duration,
    ) -> Result<ReconciliationReport> {
        let start = tokio::time::Instant::now();
        loop {
            let report = self.reconcile(client)?;
            let elapsed = start.elapsed();
            if report.is_settled() || elapsed >= timeout {
                return Ok(report);
            }
            tokio::time::sleep(interval.min(timeout.saturating_sub(elapsed))).await;
        }
    }

    /// Reconcile against an explicit snapshot of positions and working orders **without deal
    /// history**. Deterministic and I/O-free — for unit tests, offline replays, and post-restart
    /// reconstruction.
    ///
    /// Because no deal history is supplied, orders that are not found will *not* be declared
    /// absent under the default policy (`require_deal_history`). Use
    /// [`reconcile_with_deals`](Self::reconcile_with_deals) to include history.
    pub fn reconcile_with_snapshot(
        &mut self,
        live_positions: Vec<Position>,
        live_orders: Vec<WorkingOrder>,
    ) -> ReconciliationReport {
        self.reconcile_snapshot_at(
            BrokerSnapshot {
                positions: live_positions,
                pending_orders: live_orders,
                deals: None,
            },
            Utc::now().timestamp(),
        )
    }

    /// Reconcile against explicit positions, working orders **and deal history**.
    pub fn reconcile_with_deals(
        &mut self,
        live_positions: Vec<Position>,
        live_orders: Vec<WorkingOrder>,
        deals: Vec<Deal>,
    ) -> ReconciliationReport {
        self.reconcile_snapshot_at(
            BrokerSnapshot {
                positions: live_positions,
                pending_orders: live_orders,
                deals: Some(deals),
            },
            Utc::now().timestamp(),
        )
    }

    /// Core reconciliation with an explicit clock (`now`, UTC seconds), so grace-period logic
    /// is fully deterministic under test.
    pub fn reconcile_snapshot_at(
        &mut self,
        snapshot: BrokerSnapshot,
        now: i64,
    ) -> ReconciliationReport {
        self.lifecycle = LifecycleState::Reconciling;
        let BrokerSnapshot {
            positions: live_positions,
            pending_orders: live_orders,
            deals,
        } = snapshot;
        let deal_history_available = deals.is_some();
        let all_deals: &[Deal] = deals.as_deref().unwrap_or(&[]);

        let mut reconciled_orders = Vec::new();
        let mut absent_orders = Vec::new();
        let mut unresolved_orders = Vec::new();
        let mut mismatched_orders = Vec::new();
        let mut matched_position_tickets: HashSet<u64> = HashSet::new();

        // Which tracked order (if any) already owns each position / working-order ticket.
        let position_owner: HashMap<u64, String> = self
            .orders
            .values()
            .filter(|o| o.position_ticket > 0)
            .map(|o| (o.position_ticket, o.client_order_id.clone()))
            .collect();
        let order_owner: HashMap<u64, String> = self
            .orders
            .values()
            .filter(|o| o.order_ticket > 0)
            .map(|o| (o.order_ticket, o.client_order_id.clone()))
            .collect();

        let policy = self.policy.clone();

        for tracked in self.orders.values_mut() {
            if !tracked.is_unresolved() {
                continue;
            }
            let cid = tracked.client_order_id.clone();
            let owned_by_other = |owner: Option<&String>| owner.is_some_and(|o| o != &cid);

            // ---- Candidate positions: exact ticket wins; otherwise unambiguous wire-token match.
            let exact_pos = (tracked.position_ticket > 0)
                .then(|| {
                    live_positions
                        .iter()
                        .find(|p| p.ticket == tracked.position_ticket)
                })
                .flatten();
            let pos_cands: Vec<&Position> = match exact_pos {
                Some(p) => vec![p],
                None => live_positions
                    .iter()
                    .filter(|p| comment_matches_client_order_id(&p.comment, &cid))
                    .filter(|p| !owned_by_other(position_owner.get(&p.ticket)))
                    .filter(|p| !matched_position_tickets.contains(&p.ticket))
                    .collect(),
            };

            // ---- Candidate working orders.
            let exact_ord = (tracked.order_ticket > 0)
                .then(|| {
                    live_orders
                        .iter()
                        .find(|o| o.ticket == tracked.order_ticket)
                })
                .flatten();
            let ord_cands: Vec<&WorkingOrder> = match exact_ord {
                Some(o) => vec![o],
                None => live_orders
                    .iter()
                    .filter(|o| comment_matches_client_order_id(&o.comment, &cid))
                    .filter(|o| !owned_by_other(order_owner.get(&o.ticket)))
                    .collect(),
            };

            // ---- Entry deals attributed to this order (deal history).
            let entry_deals: Vec<&Deal> = all_deals
                .iter()
                .filter(|d| d.entry.is_entry())
                .filter(|d| {
                    (tracked.order_ticket > 0 && d.order == tracked.order_ticket)
                        || comment_matches_client_order_id(&d.comment, &cid)
                })
                .collect();

            // Never guess between several equally plausible candidates.
            if pos_cands.len() > 1 || ord_cands.len() > 1 {
                warn!(
                    client_order_id = %cid,
                    positions = pos_cands.len(),
                    orders = ord_cands.len(),
                    "Ambiguous reconciliation match; leaving order Unknown for manual review"
                );
                unresolved_orders.push(tracked.clone());
                continue;
            }

            if !entry_deals.is_empty() {
                let deal_position_id = entry_deals.iter().map(|d| d.position_id).find(|&id| id > 0);
                let pos = deal_position_id
                    .and_then(|id| live_positions.iter().find(|p| p.ticket == id))
                    .or_else(|| pos_cands.first().copied());
                if let Some(p) = pos {
                    matched_position_tickets.insert(p.ticket);
                }
                tracked.reconcile_with_deals(&entry_deals, pos, ord_cands.first().copied());
                info!(
                    client_order_id = %cid,
                    deals = entry_deals.len(),
                    filled = tracked.filled_volume,
                    state = ?tracked.state,
                    "Reconstructed unknown order from deal history"
                );
            } else if let Some(pos) = pos_cands.first() {
                matched_position_tickets.insert(pos.ticket);
                tracked.reconcile_with_position(pos);
                info!(
                    client_order_id = %cid,
                    ticket = pos.ticket,
                    state = ?tracked.state,
                    "Reconciled unknown order against live position"
                );
            } else if let Some(ord) = ord_cands.first() {
                tracked.reconcile_with_working_order(ord);
                info!(
                    client_order_id = %cid,
                    ticket = ord.ticket,
                    state = ?tracked.state,
                    "Reconciled unknown order against live pending order"
                );
            } else {
                // ---- Not found anywhere in this snapshot.
                // Profile matching: if the broker stripped or modified comments,
                // check if an unowned candidate matches symbol, side, volume, and time window.
                let profile_match = live_positions.iter().any(|p| {
                    !owned_by_other(position_owner.get(&p.ticket))
                        && p.symbol.eq_ignore_ascii_case(&tracked.symbol)
                        && (tracked.order_type.is_buy() == (p.position_type == OrderType::Buy))
                        && (p.volume - tracked.requested_volume).abs() < 1e-6
                        && (p.time - tracked.created_at).abs() <= 300
                }) || live_orders.iter().any(|o| {
                    !owned_by_other(order_owner.get(&o.ticket))
                        && o.symbol.eq_ignore_ascii_case(&tracked.symbol)
                        && (tracked.order_type == o.order_type)
                        && (o.volume_initial - tracked.requested_volume).abs() < 1e-6
                        && (o.time_setup - tracked.created_at).abs() <= 300
                }) || all_deals.iter().any(|d| {
                    d.entry.is_entry()
                        && d.symbol.eq_ignore_ascii_case(&tracked.symbol)
                        && (d.volume - tracked.requested_volume).abs() < 1e-6
                        && (d.time - tracked.created_at).abs() <= 300
                });

                if profile_match {
                    warn!(
                        client_order_id = %cid,
                        "Order comment match not found, but broker has matching candidate profile(s); cannot conclude absent"
                    );
                    unresolved_orders.push(tracked.clone());
                    continue;
                }

                if policy.require_deal_history && !deal_history_available {
                    warn!(
                        client_order_id = %cid,
                        "Order not found, but no deal history was consulted; cannot conclude it was never placed"
                    );
                    unresolved_orders.push(tracked.clone());
                    continue;
                }

                tracked.absent_observations = tracked.absent_observations.saturating_add(1);
                let first = *tracked.first_absent_at.get_or_insert(now);
                tracked.updated_at = now;

                let enough_observations =
                    tracked.absent_observations >= policy.min_absent_observations.max(1);
                let old_enough = now - first >= policy.min_absent_secs;
                if enough_observations && old_enough {
                    tracked.state = OrderState::Rejected;
                    tracked.rejection = Some(RejectionEvidence::ReconciledAbsent {
                        observations: tracked.absent_observations,
                        span_secs: now - first,
                    });
                    tracked.error_message = Some(format!(
                        "Reconciliation confirmed order was not placed with broker ({} absent observations over {}s)",
                        tracked.absent_observations,
                        now - first
                    ));
                    absent_orders.push(tracked.clone());
                    warn!(
                        client_order_id = %cid,
                        observations = tracked.absent_observations,
                        "Unknown order confirmed absent from broker during reconciliation"
                    );
                } else {
                    info!(
                        client_order_id = %cid,
                        observations = tracked.absent_observations,
                        required = policy.min_absent_observations,
                        "Unknown order not yet visible on broker; keeping Unknown until absence is confirmed"
                    );
                    unresolved_orders.push(tracked.clone());
                }
                continue;
            }

            match tracked.state {
                OrderState::Mismatched => {
                    warn!(
                        client_order_id = %cid,
                        mismatches = ?tracked.mismatches,
                        "Order found on broker but attributes differ from the request"
                    );
                    mismatched_orders.push(tracked.clone());
                }
                _ => reconciled_orders.push(tracked.clone()),
            }
        }

        // Identify live positions not matched to any locally tracked order
        let foreign_positions: Vec<Position> = live_positions
            .iter()
            .filter(|p| !matched_position_tickets.contains(&p.ticket))
            .filter(|p| !self.orders.values().any(|o| o.position_ticket == p.ticket))
            .cloned()
            .collect();

        let needs_attention = self
            .orders
            .values()
            .any(|o| o.is_unresolved() || o.state == OrderState::Mismatched);
        self.lifecycle = if needs_attention {
            LifecycleState::Degraded
        } else {
            LifecycleState::Ready
        };

        ReconciliationReport {
            positions: live_positions,
            pending_orders: live_orders,
            reconciled_orders,
            absent_orders,
            foreign_positions,
            timestamp: now,
            unresolved_orders,
            mismatched_orders,
            deal_history_available,
        }
    }

    /// Apply an asynchronous broker `TradeEvent` (protocol v5+ push model).
    ///
    /// Correlates the event by wire token (parsed from the event comment), position ticket,
    /// or order ticket, and updates the tracked order's state. If a durable [`OrderStore`]
    /// is configured, the updated state is immediately saved to disk.
    ///
    /// Returns the updated [`TrackedOrder`] if an active order matched the event.
    pub fn apply_trade_event(&mut self, ev: &TradeEvent) -> Result<Option<TrackedOrder>> {
        // 1. Try matching via wire token from event comment
        let matched_id = if !ev.comment.is_empty() {
            if let Some(id) = self.wire_index.get(&ev.comment).cloned() {
                Some(id)
            } else if let Some(token) = parse_wire_id(&ev.comment) {
                self.wire_index.get(token).cloned()
            } else {
                None
            }
        } else {
            None
        };

        // 2. Fallback to ticket matching
        let target_id = matched_id.or_else(|| {
            self.orders.values().find_map(|ord| {
                if (ev.position > 0 && ord.position_ticket == ev.position)
                    || (ev.order > 0 && ord.order_ticket == ev.order)
                    || (ev.deal > 0 && ord.deal_tickets.contains(&ev.deal))
                {
                    Some(ord.client_order_id.clone())
                } else {
                    None
                }
            })
        });

        if let Some(id) = target_id {
            if let Some(ord) = self.orders.get_mut(&id) {
                ord.update_from_trade_event(ev);
                let cloned = ord.clone();
                self.persist_update(&cloned)?;
                return Ok(Some(cloned));
            }
        }

        Ok(None)
    }

    /// Explicitly retry an order that is **definitively** not on the broker.
    ///
    /// This is the only sanctioned way to re-attempt an intent after a failure. It refuses
    /// unless `previous_client_order_id` is in a state that proves nothing was placed
    /// (`Rejected` or `Cancelled`); in particular an order that is `Unknown`/`Submitting` must first be
    /// resolved by [`reconcile`](Self::reconcile), and an order that is filled/working must
    /// not be duplicated. The retry gets a fresh, traceable ID (`<previous>-r<N>`), preserving
    /// idempotency for the new attempt. `req.symbol` and `req.order_type` must match the
    /// original — this is a retry, not a new order.
    pub fn retry_order<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
        previous_client_order_id: &str,
        mut req: OrderRequest,
    ) -> Result<TrackedOrder> {
        let not_allowed = |reason: String| Mt5Error::RetryNotAllowed {
            client_order_id: previous_client_order_id.to_string(),
            reason,
        };
        let prev = self
            .orders
            .get(previous_client_order_id)
            .ok_or_else(|| not_allowed("no such tracked order".to_string()))?;

        match prev.state {
            OrderState::Rejected | OrderState::Cancelled => {}
            OrderState::Unknown | OrderState::Submitting | OrderState::Created => {
                return Err(not_allowed(format!(
                    "its broker-side outcome is unresolved ({:?}); reconcile until it is resolved before retrying",
                    prev.state
                )));
            }
            other => {
                return Err(not_allowed(format!(
                    "it is already {other:?} on the broker; retrying would duplicate exposure"
                )));
            }
        }
        if !prev.symbol.eq_ignore_ascii_case(&req.symbol) || prev.order_type != req.order_type {
            return Err(not_allowed(
                "retry must keep the original symbol and order type".to_string(),
            ));
        }

        let prefix = format!("{previous_client_order_id}-r");
        let attempt = self
            .orders
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .count()
            + 1;
        let new_id = format!("{prefix}{attempt}");
        if new_id.len() > MAX_CLIENT_ORDER_ID_BYTES {
            return Err(not_allowed(format!(
                "derived retry ID would exceed {MAX_CLIENT_ORDER_ID_BYTES} bytes; supply a shorter original ID"
            )));
        }
        req.client_order_id = Some(new_id);
        self.submit_order(client, req)
    }

    /// Operator acknowledgement of a `Mismatched` order: accept the broker-side reality
    /// as the truth and move the order to `Reconciled` (or `Accepted` if nothing has filled).
    pub fn acknowledge_mismatch(&mut self, client_order_id: &str) -> Result<TrackedOrder> {
        let order = self.orders.get_mut(client_order_id).ok_or_else(|| {
            Mt5Error::ReconciliationError(format!("no tracked order '{client_order_id}'"))
        })?;
        if order.state != OrderState::Mismatched {
            return Err(Mt5Error::ReconciliationError(format!(
                "order '{client_order_id}' is {:?}, not Mismatched",
                order.state
            )));
        }
        warn!(
            client_order_id = %client_order_id,
            mismatches = ?order.mismatches,
            "Mismatch acknowledged by operator"
        );
        order.state = if order.filled_volume > 0.0 || order.position_ticket > 0 {
            OrderState::Reconciled
        } else {
            OrderState::Accepted
        };
        order.mismatches.clear();
        order.updated_at = chrono::Utc::now().timestamp();
        let out = order.clone();
        self.persist()?;
        Ok(out)
    }

    /// Close an existing position owned by this strategy.
    ///
    /// If the close request fails ambiguously (e.g. pipe timeout or lost response),
    /// the tracked order transitions to [`OrderState::Unknown`] and the manager lifecycle
    /// degrades to [`LifecycleState::Degraded`] to prevent duplicate order activity
    /// until reconciled against broker deals and open positions.
    pub fn close_position<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
        ticket: u64,
    ) -> Result<TradeResult> {
        let outcome = client.order_close_with_magic(ticket, self.strategy_magic.get());
        match outcome {
            Ok(res) => {
                let mut touched = false;
                for o in self.orders.values_mut() {
                    if o.position_ticket == ticket || o.order_ticket == ticket {
                        o.state = OrderState::Cancelled;
                        o.updated_at = chrono::Utc::now().timestamp();
                        touched = true;
                    }
                }
                if touched {
                    for o in self.orders.values() {
                        if o.position_ticket == ticket || o.order_ticket == ticket {
                            if let Err(e) = self.persist_update(o) {
                                error!(ticket, error = %e, "Failed to journal position close");
                            }
                        }
                    }
                }
                Ok(res)
            }
            Err(e) => {
                let (new_state, _evidence, desc) = classify_send_error(&e);
                if new_state == OrderState::Unknown {
                    let mut touched = false;
                    for o in self.orders.values_mut() {
                        if o.position_ticket == ticket || o.order_ticket == ticket {
                            o.mark_unknown(&desc);
                            touched = true;
                        }
                    }
                    self.lifecycle = LifecycleState::Degraded;
                    warn!(
                        ticket,
                        "close_position execution outcome is UNKNOWN; degraded lifecycle until reconciled"
                    );
                    if touched {
                        for o in self.orders.values() {
                            if o.position_ticket == ticket || o.order_ticket == ticket {
                                if let Err(pe) = self.persist_update(o) {
                                    error!(ticket, error = %pe, "Failed to journal unknown close state");
                                }
                            }
                        }
                    }
                }
                Err(e)
            }
        }
    }

    /// Modify Stop Loss / Take Profit for a position or pending order owned by this strategy.
    ///
    /// On success the tracked order for that ticket (if any) has its requested protection
    /// updated, so a later reconciliation does not flag the deliberate change as a mismatch.
    /// If the modify request fails with an ambiguous outcome, the order is marked
    /// as [`OrderState::Unknown`] and the lifecycle degrades to [`LifecycleState::Degraded`].
    pub fn modify_order<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
        ticket: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        let outcome = client.order_modify_with_magic(
            ticket,
            self.strategy_magic.get(),
            stop_loss,
            take_profit,
        );
        match outcome {
            Ok(res) => {
                let mut touched = false;
                for o in self.orders.values_mut() {
                    if o.position_ticket == ticket || o.order_ticket == ticket {
                        match stop_loss {
                            Level::Set(v) => o.requested_stop_loss = v,
                            Level::Clear => o.requested_stop_loss = 0.0,
                            Level::Keep => {}
                        }
                        match take_profit {
                            Level::Set(v) => o.requested_take_profit = v,
                            Level::Clear => o.requested_take_profit = 0.0,
                            Level::Keep => {}
                        }
                        o.updated_at = chrono::Utc::now().timestamp();
                        touched = true;
                    }
                }
                if touched {
                    for o in self.orders.values() {
                        if o.position_ticket == ticket || o.order_ticket == ticket {
                            if let Err(e) = self.persist_update(o) {
                                error!(ticket, error = %e, "Failed to journal SL/TP update");
                            }
                        }
                    }
                }
                Ok(res)
            }
            Err(e) => {
                let (new_state, _evidence, desc) = classify_send_error(&e);
                if new_state == OrderState::Unknown {
                    let mut touched = false;
                    for o in self.orders.values_mut() {
                        if o.position_ticket == ticket || o.order_ticket == ticket {
                            o.mark_unknown(format!("modify_order ambiguous state: {desc}"));
                            touched = true;
                        }
                    }
                    self.lifecycle = LifecycleState::Degraded;
                    warn!(
                        ticket,
                        "modify_order execution outcome is UNKNOWN; degraded lifecycle until reconciled"
                    );
                    if touched {
                        for o in self.orders.values() {
                            if o.position_ticket == ticket || o.order_ticket == ticket {
                                if let Err(pe) = self.persist_update(o) {
                                    error!(ticket, error = %pe, "Failed to journal unknown modify state");
                                }
                            }
                        }
                    }
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_magic_validation_and_conversions() {
        assert!(StrategyMagic::new(0).is_err());
        assert!(StrategyMagic::const_new(0).is_none());
        assert!(StrategyMagic::try_from(0).is_err());

        let magic = StrategyMagic::new(20240101).expect("valid magic");
        assert_eq!(magic.get(), 20240101);
        assert_eq!(magic.as_non_zero().get(), 20240101);
        assert_eq!(u64::from(magic), 20240101);
        assert_eq!(NonZeroU64::from(magic).get(), 20240101);

        let const_magic = StrategyMagic::const_new(20240101).expect("valid const magic");
        assert_eq!(const_magic, magic);
        assert_eq!(magic, 20240101);
        assert_eq!(20240101, magic);

        let from_nz = StrategyMagic::from(NonZeroU64::new(20240101).unwrap());
        assert_eq!(from_nz, magic);

        let from_try = StrategyMagic::try_from(20240101).expect("try_from valid");
        assert_eq!(from_try, magic);

        assert_eq!(format!("{magic}"), "20240101");

        let serialized = serde_json::to_string(&magic).expect("serialize");
        assert_eq!(serialized, "20240101");
        let deserialized: StrategyMagic =
            serde_json::from_str(&serialized).expect("deserialize valid");
        assert_eq!(deserialized, magic);
        assert!(serde_json::from_str::<StrategyMagic>("0").is_err());
    }

    #[test]
    fn constructors_reject_zero_without_panicking() {
        let unwind_try_new = std::panic::catch_unwind(|| OrderManager::try_new(0));
        assert!(unwind_try_new.is_ok());
        let res_try_new = unwind_try_new.unwrap();
        assert!(matches!(res_try_new, Err(Mt5Error::InvalidRequest(_))));

        let unwind_try_policy =
            std::panic::catch_unwind(|| OrderManager::try_with_policy(0, SafetyPolicy::default()));
        assert!(unwind_try_policy.is_ok());
        let res_try_policy = unwind_try_policy.unwrap();
        assert!(matches!(res_try_policy, Err(Mt5Error::InvalidRequest(_))));

        let unwind_magic_new = std::panic::catch_unwind(|| StrategyMagic::new(0));
        assert!(unwind_magic_new.is_ok());
        let res_magic_new = unwind_magic_new.unwrap();
        assert!(matches!(res_magic_new, Err(Mt5Error::InvalidRequest(_))));

        struct DummyStore;
        impl OrderStore for DummyStore {
            fn save(&self, _orders: &[TrackedOrder]) -> Result<()> {
                Ok(())
            }
            fn load(&self) -> Result<Vec<TrackedOrder>> {
                Ok(vec![])
            }
        }

        let unwind_store =
            std::panic::catch_unwind(|| OrderManager::with_store(0, Box::new(DummyStore)));
        assert!(unwind_store.is_ok());
        let res_store = unwind_store.unwrap();
        assert!(matches!(res_store, Err(Mt5Error::InvalidRequest(_))));
    }

    #[test]
    fn valid_constructors_succeed() {
        let magic = StrategyMagic::new(20240101).expect("valid magic");

        let mgr1 = OrderManager::new(magic);
        assert_eq!(mgr1.strategy_magic(), 20240101);
        assert_eq!(mgr1.magic(), magic);

        let mgr2 = OrderManager::try_new(20240101).expect("try_new");
        assert_eq!(mgr2.strategy_magic(), 20240101);
        assert_eq!(mgr2.magic(), magic);

        let mgr3 = OrderManager::with_policy(magic, SafetyPolicy::default());
        assert_eq!(mgr3.strategy_magic(), 20240101);

        let mgr4 = OrderManager::try_with_policy(20240101, SafetyPolicy::default())
            .expect("try_with_policy");
        assert_eq!(mgr4.strategy_magic(), 20240101);
    }

    #[test]
    fn submit_order_magic_defaults_to_strategy_magic() {
        let manager = OrderManager::try_new(20240101).expect("valid manager");
        let mut req = OrderRequest::buy("EURUSD", 0.1);
        assert_eq!(req.magic, None);
        let res = manager.check_idempotency_and_magic(&mut req);
        assert!(res.is_ok());
        assert_eq!(req.magic, Some(20240101));
    }

    #[test]
    fn submit_order_magic_rejects_mismatched_magic() {
        let manager = OrderManager::try_new(20240101).expect("valid manager");
        let mut req = OrderRequest::buy("EURUSD", 0.1).magic(999999);
        let res = manager.check_idempotency_and_magic(&mut req);
        match res {
            Err(Mt5Error::OwnershipMismatch {
                expected_magic,
                actual_magic,
                ..
            }) => {
                assert_eq!(expected_magic, 20240101);
                assert_eq!(actual_magic, 999999);
            }
            other => panic!("expected OwnershipMismatch, got {other:?}"),
        }
    }
}

//! High-level Order Manager and Reconciliation Engine.
//!
//! Provides:
//! - Full order lifecycle tracking via [`OrderManager`].
//! - Explicit order idempotency guarantees via client order IDs, with collision-safe identity
//!   (see [`wire_id`]).
//! - Post-disconnect and startup broker reconciliation via [`ReconciliationReport`] that
//!   consults **positions, working orders and deal history**, compares attributes rather than
//!   mere existence, and requires *repeated* absence before declaring an order unplaced.
//! - Strict strategy ownership enforcement via magic numbers.
//! - First-class handling of uncertain execution (`OrderState::Unknown`), including a
//!   submission gate that stops naive retries while any outcome is unresolved.
//! - Optional durable write-ahead journaling ([`OrderStore`]).
//! - A thread-safe wrapper ([`SharedOrderManager`]) that structurally enforces the locking
//!   discipline the idempotency guarantee depends on.
//!
//! # The core invariant
//!
//! ```text
//! request outcome unknown  =>  do not retry  =>  reconcile  =>  determine broker state
//! ```
//!
//! # Concurrency contract
//!
//! [`OrderManager`] has **no internal synchronization**: every mutating method takes
//! `&mut self`. The idempotency guarantee ("a `client_order_id` reaches the broker at most
//! once") holds only if *check → mark-submitting → transmit → record-outcome* happens
//! atomically with respect to every other call on the same logical order book.
//!
//! * Single thread / single owner: nothing to do; the borrow checker enforces it.
//! * Multiple threads: use [`SharedOrderManager`], which holds one lock across the entire
//!   `submit_order` call. Do **not** hand-roll a `Mutex<OrderManager>` that is released
//!   between the idempotency check and the network call, and do **not** run several
//!   `OrderManager` instances for one logical strategy — either silently forfeits the
//!   duplicate-submission protection. (`tests/failure_injection.rs` demonstrates both
//!   failure modes.)

use crate::backend::TradingBackend;
use crate::error::{Mt5Error, Result};
use crate::journal::OrderStore;
use crate::types::{
    comment_matches_client_order_id, wire_id, Deal, OrderRequest, OrderState, Position,
    TrackedOrder, TradeResult, WorkingOrder, MAX_CLIENT_ORDER_ID_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

/// Slack (seconds) added on both sides of the deal-history window fetched by
/// [`OrderManager::reconcile`], to tolerate clock skew between this host and the broker.
const DEAL_WINDOW_SLACK_SECS: i64 = 900;

/// System operational and startup lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LifecycleState {
    /// System initialized, awaiting connection.
    Starting,
    /// Establishing connection to MT5 terminal/bridge.
    Connecting,
    /// Authenticating with bridge secret.
    Authenticating,
    /// Synchronizing symbol specifications and market data.
    Syncing,
    /// Reconciling open positions and orders against live broker state.
    Reconciling,
    /// Reconciled and ready to safely accept trading operations.
    Ready,
    /// Encountered communication failure, an unresolved unknown execution, or a mismatch
    /// needing review.
    Degraded,
    /// Bridge shut down.
    Stopped,
}

/// Which submissions are blocked while an order's execution outcome is unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnknownBlockScope {
    /// Any unresolved order blocks every new submission from this strategy (default; safest —
    /// after an ambiguous response the pipe is torn down anyway, and exposure is unknown).
    Strategy,
    /// An unresolved order blocks new submissions for the same symbol only.
    Symbol,
    /// Never block. A naive retry with a *new*
    /// `client_order_id` can double exposure. Not recommended.
    Disabled,
}

/// Safety knobs governing when an order's fate may be decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafetyPolicy {
    /// An `Unknown` order must be observed **absent** (no position, no working order, no
    /// deal) in at least this many separate reconciliation passes before it is declared
    /// rejected. MT5 state can become visible asynchronously after a reconnect, so a single
    /// empty snapshot proves nothing. Minimum effective value is 1. Default: 3.
    pub min_absent_observations: u32,
    /// …and the first such observation must be at least this many seconds old. Default: 30.
    pub min_absent_secs: i64,
    /// If `true`, an order is never declared absent unless the pass included deal history
    /// (an order can fill and its position close entirely between two polls, leaving neither
    /// position nor working order behind). Default: `true`.
    pub require_deal_history: bool,
    /// Which submissions to block while any order is unresolved. Default: [`UnknownBlockScope::Strategy`].
    pub unknown_block_scope: UnknownBlockScope,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self {
            min_absent_observations: 3,
            min_absent_secs: 30,
            require_deal_history: true,
            unknown_block_scope: UnknownBlockScope::Strategy,
        }
    }
}

/// Everything the broker reported at one instant, as input to a reconciliation pass.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrokerSnapshot {
    pub positions: Vec<Position>,
    pub pending_orders: Vec<WorkingOrder>,
    /// Trade history covering the window of interest, or `None` if it could not be
    /// obtained. `None` is *not* the same as an empty list: an empty list asserts "nothing
    /// executed in this window".
    pub deals: Option<Vec<Deal>>,
}

/// Comprehensive report produced by a reconciliation cycle.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReconciliationReport {
    /// All active open positions matching strategy magic number.
    pub positions: Vec<Position>,
    /// All working pending orders matching strategy magic number.
    pub pending_orders: Vec<WorkingOrder>,
    /// Orders that were resolved from `Unknown` or `Submitting` to a verified
    /// `Reconciled` / `Accepted` / `PartiallyFilled` state with no attribute differences.
    pub reconciled_orders: Vec<TrackedOrder>,
    /// Tracked orders that were `Unknown` and are now **confirmed** absent from MT5 (repeatedly
    /// observed, with deal history) and therefore `Rejected`.
    pub absent_orders: Vec<TrackedOrder>,
    /// Open positions found in MT5 that were not tracked by local memory.
    pub foreign_positions: Vec<Position>,
    /// UTC timestamp of the reconciliation pass.
    pub timestamp: i64,
    /// Orders that are **still `Unknown`** after this pass: not found anywhere yet but not
    /// absent long enough / often enough to be declared unplaced, ambiguous, or lacking deal
    /// history. They keep blocking new submissions. Reconcile again later.
    #[serde(default)]
    pub unresolved_orders: Vec<TrackedOrder>,
    /// Orders that were found on the broker but whose volume, direction, protection, symbol or
    /// magic differ from the request (state `Mismatched`; see [`TrackedOrder::mismatches`]).
    #[serde(default)]
    pub mismatched_orders: Vec<TrackedOrder>,
    /// Whether this pass had deal history to consult.
    #[serde(default)]
    pub deal_history_available: bool,
}

impl ReconciliationReport {
    /// Returns true if local and broker state agree completely: no confirmed-absent orders,
    /// no foreign positions, nothing unresolved and nothing mismatched.
    pub fn is_clean(&self) -> bool {
        self.absent_orders.is_empty()
            && self.foreign_positions.is_empty()
            && self.unresolved_orders.is_empty()
            && self.mismatched_orders.is_empty()
    }

    /// `true` if every previously uncertain order now has a definite outcome.
    pub fn is_settled(&self) -> bool {
        self.unresolved_orders.is_empty()
    }
}

static ID_SEQ: AtomicU64 = AtomicU64::new(0);

/// High-level execution and order lifecycle manager.
///
/// Wraps a [`TradingBackend`] (normally [`Mt5Client`](crate::Mt5Client)) with:
/// - Order idempotency: ensures repeated submits with the same `client_order_id` do not create duplicate exposure.
/// - Explicit order state machine: tracks volume and states (`Submitting`, `Filled`, `Partial`, `Unknown`).
/// - Post-failure reconciliation: queries live broker state and resolves ambiguous execution outcomes.
/// - Strategy ownership: enforces strategy magic number on all close, modify, and query actions.
/// - An Unknown-execution submission gate and write-ahead journaling (see the module docs).
///
/// See the [module-level concurrency contract](self#concurrency-contract): this type is
/// single-owner; use [`SharedOrderManager`] to share it between threads.
pub struct OrderManager {
    strategy_magic: u64,
    lifecycle: LifecycleState,
    orders: BTreeMap<String, TrackedOrder>,
    /// wire token → full `client_order_id`; detects hash collisions between distinct IDs.
    wire_index: HashMap<String, String>,
    policy: SafetyPolicy,
    store: Option<Box<dyn OrderStore>>,
}

impl OrderManager {
    /// Create a new `OrderManager` bound to a specific strategy magic number, with no
    /// durable journal (state lives in memory only — see [`with_store`](Self::with_store)).
    pub fn new(strategy_magic: u64) -> Self {
        Self {
            strategy_magic,
            lifecycle: LifecycleState::Starting,
            orders: BTreeMap::new(),
            wire_index: HashMap::new(),
            policy: SafetyPolicy::default(),
            store: None,
        }
    }

    /// Create a manager with an explicit [`SafetyPolicy`].
    pub fn with_policy(strategy_magic: u64, policy: SafetyPolicy) -> Self {
        let mut m = Self::new(strategy_magic);
        m.policy = policy;
        m
    }

    /// Create a manager backed by a durable [`OrderStore`], restoring any state it holds.
    ///
    /// From here on every state change is journaled, and — critically — a `Submitting`
    /// record is written and made durable **before** an order is transmitted. If the process
    /// dies mid-request, the next start finds that record, treats it as `Unknown`, and
    /// reconciliation resolves it against the broker. Orders restored in `Submitting` state
    /// are converted to `Unknown` for exactly that reason.
    ///
    /// Fails (rather than starting empty) if the store exists but cannot be read.
    pub fn with_store(strategy_magic: u64, store: Box<dyn OrderStore>) -> Result<Self> {
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

    /// Strategy magic number owned by this manager.
    pub fn strategy_magic(&self) -> u64 {
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
        let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
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

    /// Flush the current state to the configured [`OrderStore`] (no-op without one).
    pub fn persist(&self) -> Result<()> {
        match &self.store {
            Some(store) => store.save(&self.export_orders()),
            None => Ok(()),
        }
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
            req.magic = Some(self.strategy_magic);
        } else if req.magic != Some(self.strategy_magic) {
            return Err(Mt5Error::OwnershipMismatch {
                ticket: 0,
                expected_magic: self.strategy_magic,
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
    /// 2. Idempotency: a repeated `client_order_id` returns the existing record and never
    ///    re-contacts the broker.
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
        if let Err(e) = self.persist() {
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
            Err(Mt5Error::UnknownExecutionState { description, .. }) => {
                // Ambiguous network outcome: response lost after submission!
                tracked.mark_unknown(&description);
                self.lifecycle = LifecycleState::Degraded;
                warn!(
                    client_order_id = %cid,
                    "Order execution outcome is UNKNOWN! Reconcile against broker before retrying."
                );
                Err(Mt5Error::UnknownExecutionState {
                    symbol: req.symbol.clone(),
                    client_order_id: Some(cid.clone()),
                    description,
                })
            }
            Err(e) => {
                tracked.state = OrderState::Rejected;
                tracked.error_message = Some(e.to_string());
                tracked.updated_at = chrono::Utc::now().timestamp();
                Err(e)
            }
        };

        self.track_order(tracked.clone());
        if let Err(e) = self.persist() {
            // The write-ahead record (Submitting → Unknown on restore) still protects us.
            error!(client_order_id = %cid, error = %e, "Failed to journal order outcome; durable state is stale but safe");
        }

        result.map(|()| tracked)
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
    pub fn close_position<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        ticket: u64,
    ) -> Result<TradeResult> {
        client.order_close_with_magic(ticket, self.strategy_magic)
    }

    /// Modify Stop Loss / Take Profit for a position or pending order owned by this strategy.
    ///
    /// On success the tracked order for that ticket (if any) has its requested protection
    /// updated, so a later reconciliation does not flag the deliberate change as a mismatch.
    pub fn modify_order<B: TradingBackend + ?Sized>(
        &mut self,
        client: &B,
        ticket: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        let res =
            client.order_modify_with_magic(ticket, self.strategy_magic, stop_loss, take_profit)?;
        let mut touched = false;
        for o in self.orders.values_mut() {
            if o.position_ticket == ticket || o.order_ticket == ticket {
                o.requested_stop_loss = stop_loss;
                o.requested_take_profit = take_profit;
                o.updated_at = chrono::Utc::now().timestamp();
                touched = true;
            }
        }
        if touched {
            if let Err(e) = self.persist() {
                error!(ticket, error = %e, "Failed to journal SL/TP update");
            }
        }
        Ok(res)
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
        let now = chrono::Utc::now().timestamp();

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
        let positions = client.positions_filtered(Some(self.strategy_magic), None)?;
        let pending_orders = client.pending_orders_filtered(Some(self.strategy_magic), None)?;

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
                match client.deals_filtered(from, to, Some(self.strategy_magic), None) {
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
    /// `timeout` elapses, returning the last report. Blocks the calling thread.
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
            std::thread::sleep(interval.min(timeout - elapsed));
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
            chrono::Utc::now().timestamp(),
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
            chrono::Utc::now().timestamp(),
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
            let owned_by_other = |owner: Option<&String>| owner.map_or(false, |o| o != &cid);

            // ---- Candidate positions: exact ticket wins; otherwise unambiguous wire-token match.
            let exact_pos = (tracked.position_ticket > 0)
                .then(|| live_positions.iter().find(|p| p.ticket == tracked.position_ticket))
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
                .then(|| live_orders.iter().find(|o| o.ticket == tracked.order_ticket))
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
                let deal_position_id = entry_deals
                    .iter()
                    .map(|d| d.position_id)
                    .find(|&id| id > 0);
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
}

/// Thread-safe handle to an [`OrderManager`] that **structurally** enforces the locking
/// discipline the idempotency guarantee depends on.
///
/// Every operation holds a single mutex for its entire duration — for
/// [`submit_order`](Self::submit_order) that means idempotency check, write-ahead record,
/// broker call and outcome recording are one atomic step with respect to every other clone of
/// this handle. Concurrent submissions of the same `client_order_id` therefore result in
/// exactly one broker request; the others receive the existing tracked order.
///
/// Because the lock is held across the broker call, submissions are serialized. That matches
/// the underlying DLL, which already serializes all pipe I/O behind one critical section.
///
/// Cloning is cheap (`Arc`). A poisoned lock (a panic while holding it) is recovered rather
/// than propagated: state changes are simple map inserts, and the write-ahead `Submitting`
/// record means an interrupted submission is resolved by reconciliation like any other crash.
#[derive(Clone)]
pub struct SharedOrderManager {
    inner: Arc<Mutex<OrderManager>>,
}

impl SharedOrderManager {
    pub fn new(manager: OrderManager) -> Self {
        Self {
            inner: Arc::new(Mutex::new(manager)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, OrderManager> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` with exclusive access to the underlying manager.
    pub fn with<R>(&self, f: impl FnOnce(&mut OrderManager) -> R) -> R {
        f(&mut self.lock())
    }

    /// See [`OrderManager::submit_order`]. The lock is held for the whole call.
    pub fn submit_order<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        req: OrderRequest,
    ) -> Result<TrackedOrder> {
        self.lock().submit_order(client, req)
    }

    /// See [`OrderManager::retry_order`].
    pub fn retry_order<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        previous_client_order_id: &str,
        req: OrderRequest,
    ) -> Result<TrackedOrder> {
        self.lock().retry_order(client, previous_client_order_id, req)
    }

    /// See [`OrderManager::reconcile`].
    pub fn reconcile<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
    ) -> Result<ReconciliationReport> {
        self.lock().reconcile(client)
    }

    /// See [`OrderManager::close_position`].
    pub fn close_position<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        ticket: u64,
    ) -> Result<TradeResult> {
        self.lock().close_position(client, ticket)
    }

    /// See [`OrderManager::modify_order`].
    pub fn modify_order<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        ticket: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        self.lock().modify_order(client, ticket, stop_loss, take_profit)
    }

    /// Snapshot of one tracked order.
    pub fn get_order(&self, client_order_id: &str) -> Option<TrackedOrder> {
        self.lock().get_order(client_order_id).cloned()
    }

    /// Snapshot of all tracked orders.
    pub fn export_orders(&self) -> Vec<TrackedOrder> {
        self.lock().export_orders()
    }

    /// Current lifecycle state.
    pub fn lifecycle(&self) -> LifecycleState {
        self.lock().lifecycle()
    }
}

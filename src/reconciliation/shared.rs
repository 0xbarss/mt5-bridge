use crate::backend::TradingBackend;
use crate::error::Result;
use crate::reconciliation::manager::{OrderManager, StrategyMagic};
use crate::reconciliation::state::{LifecycleState, ReconciliationReport};
use crate::types::{Level, OrderRequest, TrackedOrder, TradeEvent, TradeResult};
use arc_swap::ArcSwap;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Lock-free, point-in-time view of the manager's order book and lifecycle state.
///
/// [`SharedOrderManager`] rebuilds this after every mutating operation and publishes it
/// atomically, so readers never contend with a slow broker call.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderSnapshot {
    /// Lifecycle state as of the last completed mutation.
    pub lifecycle: LifecycleState,
    /// Tracked orders keyed by `client_order_id`, ordered by that key.
    pub orders: BTreeMap<String, TrackedOrder>,
}

impl Default for OrderSnapshot {
    fn default() -> Self {
        Self {
            lifecycle: LifecycleState::Starting,
            orders: BTreeMap::new(),
        }
    }
}

impl OrderSnapshot {
    /// Snapshot of one tracked order.
    pub fn get_order(&self, client_order_id: &str) -> Option<&TrackedOrder> {
        self.orders.get(client_order_id)
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
/// # Lock-free reads
///
/// The write side still serializes, but reads do not: after every mutation the manager
/// publishes an [`OrderSnapshot`] through an [`ArcSwap`]. [`get_order`](Self::get_order),
/// [`export_orders`](Self::export_orders), [`lifecycle`](Self::lifecycle) and
/// [`snapshot`](Self::snapshot) therefore never wait for an in-flight broker call. They return
/// the state as of the last completed mutation; use [`try_get_order`](Self::try_get_order)
/// when the authoritative, current state is required and a busy lock may be reported.
///
/// Cloning is cheap (`Arc`). A poisoned lock (a panic while holding it) is recovered rather
/// than propagated: state changes are simple map inserts, and the write-ahead `Submitting`
/// record means an interrupted submission is resolved by reconciliation like any other crash.
#[derive(Clone)]
pub struct SharedOrderManager {
    inner: Arc<Mutex<OrderManager>>,
    snapshot: Arc<ArcSwap<OrderSnapshot>>,
    /// The strategy magic never changes after construction, so it is cached outside the lock.
    strategy_magic: StrategyMagic,
}

impl SharedOrderManager {
    pub fn new(manager: OrderManager) -> Self {
        let strategy_magic = manager.magic();
        let initial = OrderSnapshot {
            lifecycle: manager.lifecycle(),
            orders: manager.orders_snapshot(),
        };
        Self {
            inner: Arc::new(Mutex::new(manager)),
            snapshot: Arc::new(ArcSwap::from_pointee(initial)),
            strategy_magic,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, OrderManager> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Rebuild and atomically publish the read snapshot.
    ///
    /// Called after every mutating operation, including ones that return an error or panic: a
    /// failed broker call can still have changed tracked state (for example
    /// `Submitting` → `Unknown`).
    fn refresh_snapshot(&self) {
        // Read and publish while holding the manager lock: publishing outside it could let a
        // slower thread overwrite a newer snapshot with the state it read earlier.
        let manager = self.lock();
        let next = OrderSnapshot {
            lifecycle: manager.lifecycle(),
            orders: manager.orders_snapshot(),
        };
        self.snapshot.store(Arc::new(next));
    }

    /// Run `f` under the manager lock and refresh the read snapshot afterwards.
    ///
    /// The refresh also runs when `f` panics (the manager lock is poisoned, then recovered), so
    /// lock-free readers never observe a snapshot older than the last committed mutation.
    fn mutate<R>(&self, f: impl FnOnce(&mut OrderManager) -> R) -> R {
        // Declared first so it is dropped last; the manager guard is released before the
        // snapshot is rebuilt (the mutex is not re-entrant).
        let _refresh = SnapshotRefresh(self);
        let mut manager = self.lock();
        let out = f(&mut manager);
        drop(manager);
        out
    }

    /// Run `f` with exclusive access to the underlying manager.
    ///
    /// The read snapshot is refreshed afterwards so lock-free readers observe the mutation.
    pub fn with<R>(&self, f: impl FnOnce(&mut OrderManager) -> R) -> R {
        self.mutate(f)
    }

    /// Strategy magic number owned by the underlying order manager.
    pub fn strategy_magic(&self) -> u64 {
        self.strategy_magic.get()
    }

    /// Validated [`StrategyMagic`] owned by the underlying order manager.
    pub fn magic(&self) -> StrategyMagic {
        self.strategy_magic
    }

    /// Set current operational lifecycle state.
    pub fn set_lifecycle(&self, state: LifecycleState) {
        self.mutate(|manager| manager.set_lifecycle(state));
    }

    /// See [`OrderManager::submit_order`]. The lock is held for the whole call.
    pub fn submit_order<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        req: OrderRequest,
    ) -> Result<TrackedOrder> {
        self.mutate(|manager| manager.submit_order(client, req))
    }

    /// See [`OrderManager::retry_order`].
    pub fn retry_order<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        previous_client_order_id: &str,
        req: OrderRequest,
    ) -> Result<TrackedOrder> {
        self.mutate(|manager| manager.retry_order(client, previous_client_order_id, req))
    }

    /// See [`OrderManager::reconcile`].
    pub fn reconcile<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
    ) -> Result<ReconciliationReport> {
        self.mutate(|manager| manager.reconcile(client))
    }

    /// Call [`reconcile`](Self::reconcile) every `interval` until no order is unresolved or
    /// `timeout` elapses, returning the last report.
    ///
    /// # Blocking
    ///
    /// This method is synchronous and blocks the calling thread via [`std::thread::sleep`].
    /// In asynchronous contexts (such as Tokio runtimes), callers should use
    /// [`reconcile_until_settled_async`](Self::reconcile_until_settled_async) or run this within
    /// `tokio::task::spawn_blocking`.
    pub fn reconcile_until_settled<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        interval: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Result<ReconciliationReport> {
        let start = std::time::Instant::now();
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
    /// unresolved or `timeout` elapses, yielding cooperatively via [`tokio::time::sleep`].
    ///
    /// Note that each reconciliation pass acquires and releases the internal lock, allowing other
    /// tasks to submit or query orders between intervals. The underlying [`TradingBackend`] calls
    /// within [`reconcile`](Self::reconcile) remain synchronous.
    #[cfg(feature = "async")]
    pub async fn reconcile_until_settled_async<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        interval: std::time::Duration,
        timeout: std::time::Duration,
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

    /// See [`OrderManager::close_position`].
    pub fn close_position<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        ticket: u64,
    ) -> Result<TradeResult> {
        self.mutate(|manager| manager.close_position(client, ticket))
    }

    /// See [`OrderManager::modify_order`].
    pub fn modify_order<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        ticket: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        self.mutate(|manager| manager.modify_order(client, ticket, stop_loss, take_profit))
    }

    /// Point-in-time snapshot of all tracked orders and the lifecycle state.
    ///
    /// This is lock-free and returns the state as of the last completed mutation.
    pub fn snapshot(&self) -> Arc<OrderSnapshot> {
        self.snapshot.load_full()
    }

    /// Snapshot of one tracked order, read lock-free.
    ///
    /// Returns the state as of the last completed mutation; a mutation currently in flight
    /// (for example a slow broker call) is not reflected until it finishes.
    pub fn get_order(&self, client_order_id: &str) -> Option<TrackedOrder> {
        self.snapshot.load().get_order(client_order_id).cloned()
    }

    /// Read the authoritative latest order state without waiting for the manager lock.
    ///
    /// Unlike [`get_order`](Self::get_order), which reads the published snapshot, this consults
    /// the manager directly so it also reflects a mutation that is still in flight. Returns
    /// `None` if the order is unknown **or** the manager is currently busy with another
    /// operation; in the latter case retry, or fall back to [`get_order`](Self::get_order) for
    /// a lock-free (possibly slightly stale) view.
    pub fn try_get_order(&self, client_order_id: &str) -> Option<TrackedOrder> {
        self.inner
            .try_lock()
            .ok()
            .and_then(|manager| manager.get_order(client_order_id).cloned())
    }

    /// Snapshot of all tracked orders, read lock-free.
    pub fn export_orders(&self) -> Vec<TrackedOrder> {
        self.snapshot.load().orders.values().cloned().collect()
    }

    /// Current lifecycle state, read lock-free.
    pub fn lifecycle(&self) -> LifecycleState {
        self.snapshot.load().lifecycle
    }

    /// Apply an asynchronous broker `TradeEvent` (protocol v5+ push model).
    pub fn apply_trade_event(&self, ev: &TradeEvent) -> Result<Option<TrackedOrder>> {
        self.mutate(|manager| manager.apply_trade_event(ev))
    }

    /// Prune settled orders older than `max_age_seconds`.
    pub fn prune_settled_orders(&self, max_age_seconds: i64) -> usize {
        self.mutate(|manager| manager.prune_settled_orders(max_age_seconds))
    }
}

/// Refreshes the published read snapshot when dropped, including during unwinding.
struct SnapshotRefresh<'a>(&'a SharedOrderManager);

impl Drop for SnapshotRefresh<'_> {
    fn drop(&mut self) {
        self.0.refresh_snapshot();
    }
}

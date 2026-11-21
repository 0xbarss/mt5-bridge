use crate::backend::TradingBackend;
use crate::error::Result;
use crate::reconciliation::manager::OrderManager;
use crate::reconciliation::state::{LifecycleState, ReconciliationReport};
use crate::types::{Level, OrderRequest, TrackedOrder, TradeEvent, TradeResult};
use std::sync::{Arc, Mutex};

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

    /// Strategy magic number owned by the underlying order manager.
    pub fn strategy_magic(&self) -> u64 {
        self.lock().strategy_magic()
    }

    /// Set current operational lifecycle state.
    pub fn set_lifecycle(&self, state: LifecycleState) {
        self.lock().set_lifecycle(state);
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
        self.lock()
            .retry_order(client, previous_client_order_id, req)
    }

    /// See [`OrderManager::reconcile`].
    pub fn reconcile<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
    ) -> Result<ReconciliationReport> {
        self.lock().reconcile(client)
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
        self.lock().close_position(client, ticket)
    }

    /// See [`OrderManager::modify_order`].
    pub fn modify_order<B: TradingBackend + ?Sized>(
        &self,
        client: &B,
        ticket: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        self.lock()
            .modify_order(client, ticket, stop_loss, take_profit)
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

    /// Apply an asynchronous broker `TradeEvent` (protocol v5+ push model).
    pub fn apply_trade_event(&self, ev: &TradeEvent) -> Result<Option<TrackedOrder>> {
        self.lock().apply_trade_event(ev)
    }

    /// Prune settled orders older than `max_age_seconds`.
    pub fn prune_settled_orders(&self, max_age_seconds: i64) -> usize {
        self.lock().prune_settled_orders(max_age_seconds)
    }
}

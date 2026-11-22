//! Automatic reconciliation scheduler.
//!
//! Runs periodic reconciliation passes and reacts to triggers (orders that
//! transition to [`crate::types::OrderState::Unknown`] and trade `event_seq` gaps) so that
//! uncertain execution outcomes are resolved without user intervention.
//!
//! Each pass runs [`SharedOrderManager::reconcile`] via [`tokio::task::spawn_blocking`]
//! so the synchronous backend calls never block the Tokio executor. Reports are
//! emitted on a broadcast channel; subscribe via [`ReconcilerHandle::subscribe`].
//! Call [`ReconcilerHandle::stop`] to shut down the background task.

use crate::backend::TradingBackend;
use crate::error::Mt5Error;
use crate::reconciliation::state::ReconciliationReport;
use crate::reconciliation::SharedOrderManager;
use crate::types::mismatch::MismatchKind;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, warn};

/// Auto-acknowledge policy for `Mismatched` orders found during a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AutoAckPolicy {
    /// Never auto-acknowledge; mismatches require operator action
    /// ([`OrderManager::acknowledge_mismatch`](crate::reconciliation::OrderManager::acknowledge_mismatch)).
    #[default]
    Never,
    /// Auto-acknowledge only SL/TP mismatches (deliberate protection changes).
    ProtectionOnly,
    /// Auto-acknowledge all mismatches.
    All,
}

/// Configuration for [`spawn_reconciler`].
#[derive(Debug, Clone)]
pub struct ReconcilerConfig {
    /// Period between automatic reconciliation passes.
    pub interval: Duration,
    /// Trigger an immediate pass when an order transitions to `Unknown`.
    ///
    /// Wired by the caller: subscribe to order lifecycle events and call
    /// [`ReconcilerHandle::trigger`] on [`OrderEvent::Unknown`](crate::reconciliation::events::OrderEvent::Unknown).
    pub on_unknown: bool,
    /// Trigger an immediate pass when a trade `event_seq` gap is detected.
    ///
    /// Wired by the caller: call [`ReconcilerHandle::trigger`] when an
    /// `event_seq` gap is observed.
    pub on_event_gap: bool,
    /// Auto-acknowledge policy for mismatches discovered during a pass.
    pub auto_acknowledge: AutoAckPolicy,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            on_unknown: true,
            on_event_gap: true,
            auto_acknowledge: AutoAckPolicy::Never,
        }
    }
}

/// Handle to a running reconciler.
///
/// Cloneable and cheap (`Arc`-free handle over a few senders). Use
/// [`subscribe`](Self::subscribe) to receive reports, [`trigger`](Self::trigger)
/// to request an immediate pass, and [`stop`](Self::stop) to shut the task down.
#[derive(Clone, Debug)]
pub struct ReconcilerHandle {
    /// Request an immediate reconciliation pass.
    trigger_tx: mpsc::Sender<()>,
    /// Shut the background task down.
    stop_tx: mpsc::Sender<()>,
    /// Publish reconciliation reports.
    report_tx: broadcast::Sender<ReconciliationReport>,
}

impl ReconcilerHandle {
    /// Subscribe to reconciliation reports produced by this reconciler.
    pub fn subscribe(&self) -> broadcast::Receiver<ReconciliationReport> {
        self.report_tx.subscribe()
    }

    /// Trigger an immediate reconciliation pass (non-blocking; dropped if the
    /// internal channel is full).
    pub fn trigger(&self) {
        let _ = self.trigger_tx.try_send(());
    }

    /// Stop the reconciler background task.
    pub async fn stop(self) {
        let _ = self.stop_tx.send(()).await;
    }
}

/// Spawn a background task that reconciles orders automatically.
///
/// A periodic [`tokio::time::interval`] drives scheduled passes, and a trigger
/// channel drives immediate passes. The trigger channel is wired by the caller
/// (for example the gateway wires `Unknown` order events and trade `event_seq`
/// gaps to [`ReconcilerHandle::trigger`] when the corresponding config flags are
/// set).
///
/// Each pass runs [`SharedOrderManager::reconcile`] via
/// [`tokio::task::spawn_blocking`] so the synchronous backend calls never block
/// the Tokio executor. Reports are emitted on a broadcast channel; subscribe via
/// [`ReconcilerHandle::subscribe`]. Call [`ReconcilerHandle::stop`] to shut down
/// the task.
///
/// # Panics
///
/// Panics if Tokio's `time` or `rt` features are not enabled.
pub fn spawn_reconciler<B>(
    manager: SharedOrderManager,
    backend: Arc<B>,
    config: ReconcilerConfig,
) -> ReconcilerHandle
where
    B: TradingBackend + Send + Sync + 'static,
{
    let (trigger_tx, mut trigger_rx) = mpsc::channel(32);
    let (stop_tx, mut stop_rx) = mpsc::channel(1);
    let (report_tx, _report_rx) = broadcast::channel(256);

    let report_tx_clone = report_tx.clone();
    let auto_ack = config.auto_acknowledge;

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Run an initial pass promptly so startup state is reconciled without
        // waiting for the first full interval.
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let _ = run_pass(
                        &manager,
                        &backend,
                        &report_tx_clone,
                        auto_ack,
                    )
                    .await;
                }
                _ = trigger_rx.recv() => {
                    let _ = run_pass(
                        &manager,
                        &backend,
                        &report_tx_clone,
                        auto_ack,
                    )
                    .await;
                }
                _ = stop_rx.recv() => {
                    debug!("Reconciler stopped");
                    break;
                }
                else => {
                    break;
                }
            }
        }
    });

    ReconcilerHandle {
        trigger_tx,
        report_tx,
        stop_tx,
    }
}

/// Run one reconciliation pass and, when configured, auto-acknowledge mismatches.
async fn run_pass<B: TradingBackend + Send + Sync + 'static>(
    manager: &SharedOrderManager,
    backend: &Arc<B>,
    report_tx: &broadcast::Sender<ReconciliationReport>,
    auto_ack: AutoAckPolicy,
) {
    let (report, mismatched) = match tokio::task::spawn_blocking({
        let manager = manager.clone();
        let backend = backend.clone();
        move || {
            let report = manager.reconcile(&*backend)?;
            let mismatched: Vec<(String, Vec<MismatchKind>)> = report
                .mismatched_orders
                .iter()
                .map(|o| {
                    (
                        o.client_order_id.clone(),
                        o.mismatches.iter().map(|m| m.kind).collect(),
                    )
                })
                .collect();
            Ok::<(ReconciliationReport, Vec<(String, Vec<MismatchKind>)>), Mt5Error>((
                report, mismatched,
            ))
        }
    })
    .await
    {
        Ok(Ok((report, mismatched))) => (report, mismatched),
        Ok(Err(e)) => {
            warn!(error = %e, "Automatic reconciliation pass failed");
            return;
        }
        Err(e) => {
            warn!(error = %e, "Automatic reconciliation pass join failed");
            return;
        }
    };

    let _ = report_tx.send(report);

    // Auto-acknowledge mismatches per policy.
    if auto_ack != AutoAckPolicy::Never {
        for (cid, kinds) in mismatched {
            let ack = match auto_ack {
                AutoAckPolicy::ProtectionOnly => kinds
                    .iter()
                    .any(|k| matches!(k, MismatchKind::StopLoss | MismatchKind::TakeProfit)),
                AutoAckPolicy::All => true,
                AutoAckPolicy::Never => false,
            };
            if ack {
                match manager.acknowledge_mismatch(&cid) {
                    Ok(_) => {
                        debug!(client_order_id = %cid, "Auto-acknowledged mismatch during reconciliation");
                    }
                    Err(e) => {
                        warn!(
                            client_order_id = %cid,
                            error = %e,
                            "Auto-acknowledge mismatch failed",
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Mt5Error, Result};
    use crate::reconciliation::state::SafetyPolicy;
    use crate::reconciliation::OrderManager;
    use crate::types::{
        Deal, Level, OrderRequest, OrderState, OrderType, Position, TrackedOrder, TradeResult,
        WorkingOrder,
    };
    use chrono::Utc;
    use std::sync::Arc;
    use tokio::time;

    const MAGIC: u64 = 424242;

    /// Minimal simulated backend for scheduler tests.
    struct TestBackend {
        positions: std::sync::Mutex<Vec<Position>>,
        pending: std::sync::Mutex<Vec<WorkingOrder>>,
        deals: std::sync::Mutex<Option<Vec<Deal>>>,
    }

    impl TestBackend {
        fn new() -> Self {
            Self {
                positions: std::sync::Mutex::new(Vec::new()),
                pending: std::sync::Mutex::new(Vec::new()),
                deals: std::sync::Mutex::new(None),
            }
        }

        fn with_position(self, pos: Position) -> Self {
            self.positions.lock().unwrap().push(pos);
            self
        }
    }

    impl TradingBackend for TestBackend {
        fn order_send(&self, _req: &OrderRequest) -> Result<TradeResult> {
            Err(Mt5Error::Other("test backend: order_send not used".into()))
        }

        fn order_close_with_magic(&self, _ticket: u64, _magic: u64) -> Result<TradeResult> {
            Err(Mt5Error::Other("test backend: close not used".into()))
        }

        fn order_modify_with_magic(
            &self,
            _ticket: u64,
            _magic: u64,
            _stop_loss: Level,
            _take_profit: Level,
        ) -> Result<TradeResult> {
            Err(Mt5Error::Other("test backend: modify not used".into()))
        }

        fn positions_filtered(
            &self,
            _magic: Option<u64>,
            _symbol: Option<&str>,
        ) -> Result<Vec<Position>> {
            Ok(self.positions.lock().unwrap().clone())
        }

        fn pending_orders_filtered(
            &self,
            _magic: Option<u64>,
            _symbol: Option<&str>,
        ) -> Result<Vec<WorkingOrder>> {
            Ok(self.pending.lock().unwrap().clone())
        }

        fn deals_filtered(
            &self,
            _from: i64,
            _to: i64,
            _magic: Option<u64>,
            _symbol: Option<&str>,
        ) -> Result<Vec<Deal>> {
            Ok(self.deals.lock().unwrap().clone().unwrap_or_default())
        }
    }

    /// Build an `OrderManager` with one tracked order in `Unknown` state whose
    /// broker outcome is resolved by a live position matching `position_ticket`.
    fn unknown_manager(cid: &str, position_ticket: u64) -> (SharedOrderManager, String) {
        let req = OrderRequest::buy("EURUSD", 0.5).magic(MAGIC);
        let mut order = TrackedOrder::new(&req, cid);
        order.position_ticket = position_ticket;
        order.magic = MAGIC;
        order.state = OrderState::Unknown;
        order.error_message = Some("simulated lost response".into());
        let mut manager = OrderManager::try_new(MAGIC).expect("valid magic");
        manager.track_order(order);
        (SharedOrderManager::new(manager), cid.to_string())
    }

    fn position(ticket: u64, sl: f64, tp: f64, volume: f64) -> Position {
        Position {
            ticket,
            time: Utc::now().timestamp(),
            position_type: OrderType::Buy,
            magic: MAGIC,
            volume,
            price_open: 1.0850,
            stop_loss: sl,
            take_profit: tp,
            price_current: 1.0850,
            profit: 0.0,
            swap: 0.0,
            symbol: "EURUSD".into(),
            comment: String::new(),
        }
    }

    #[tokio::test]
    async fn trigger_runs_immediate_pass_and_resolves_unknown_order() {
        time::pause();
        let (manager, cid) = unknown_manager("ord-1", 5001);
        let backend = Arc::new(TestBackend::new().with_position(position(5001, 0.0, 0.0, 0.5)));
        let config = ReconcilerConfig::default();
        let handle = spawn_reconciler(manager.clone(), Arc::clone(&backend), config);
        let mut report_rx = handle.subscribe();

        handle.trigger();

        // With time paused the interval never fires; only the trigger drives a pass.
        let Ok(report) = report_rx.recv().await else {
            panic!("expected a reconciliation report after trigger");
        };
        assert_eq!(report.reconciled_orders.len(), 1);
        assert_eq!(
            manager.get_order(&cid).unwrap().state,
            OrderState::Reconciled,
            "Unknown order must be resolved by the triggered pass"
        );
        assert!(report.unresolved_orders.is_empty());
        handle.stop().await;
    }

    #[tokio::test]
    async fn periodic_pass_resolves_unknown_order() {
        // Real clock: let at least one interval elapse so the periodic pass fires.
        let (manager, cid) = unknown_manager("ord-2", 5002);
        let backend = Arc::new(TestBackend::new().with_position(position(5002, 0.0, 0.0, 0.5)));
        let config = ReconcilerConfig {
            interval: std::time::Duration::from_millis(5),
            ..Default::default()
        };
        let handle = spawn_reconciler(manager.clone(), Arc::clone(&backend), config);
        let _ = handle.subscribe();

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        assert!(
            !manager.get_order(&cid).is_some_and(|o| o.is_unresolved()),
            "periodic pass must resolve the Unknown order"
        );
        handle.stop().await;
    }

    #[tokio::test]
    async fn auto_ack_protection_only_acks_sl_tp_mismatches_only() {
        // Order A: stop loss mismatch → ProtectionOnly acks it.
        let (manager_a, cid_a) = unknown_manager("ord-a", 6001);
        // Order B: volume mismatch → ProtectionOnly does NOT ack it.
        let (manager_b, cid_b) = unknown_manager("ord-b", 6002);

        let backend_a =
            Arc::new(TestBackend::new().with_position(position(6001, 1.0900, 0.0, 0.5)));
        let backend_b = Arc::new(TestBackend::new().with_position(position(6002, 0.0, 0.0, 1.0)));

        let (report_tx, _report_rx) = broadcast::channel(16);

        // Manager A: SL differs from request (req SL 0.0, pos SL 1.0900) → StopLoss mismatch.
        run_pass(
            &manager_a,
            &backend_a,
            &report_tx,
            AutoAckPolicy::ProtectionOnly,
        )
        .await;
        assert_eq!(
            manager_a.get_order(&cid_a).unwrap().state,
            OrderState::Reconciled,
            "SL mismatch must be auto-acknowledged under ProtectionOnly"
        );

        // Manager B: volume differs from request (req 0.5, pos 1.0) → Volume mismatch, not acked.
        run_pass(
            &manager_b,
            &backend_b,
            &report_tx,
            AutoAckPolicy::ProtectionOnly,
        )
        .await;
        assert_eq!(
            manager_b.get_order(&cid_b).unwrap().state,
            OrderState::Mismatched,
            "Volume mismatch must NOT be auto-acknowledged under ProtectionOnly"
        );
    }

    #[tokio::test]
    async fn auto_ack_all_acks_every_mismatch() {
        let (manager, cid) = unknown_manager("ord-c", 6003);
        let backend = Arc::new(TestBackend::new().with_position(position(6003, 0.0, 0.0, 1.0))); // volume mismatch
        let (report_tx, _report_rx) = broadcast::channel(16);

        run_pass(&manager, &backend, &report_tx, AutoAckPolicy::All).await;
        assert!(
            !manager.get_order(&cid).unwrap().is_unresolved(),
            "All policy must acknowledge the mismatch and resolve the order"
        );
        assert_eq!(
            manager.get_order(&cid).unwrap().state,
            OrderState::Reconciled,
        );
    }

    #[tokio::test]
    async fn auto_ack_never_acks_nothing() {
        let (manager, cid) = unknown_manager("ord-d", 6004);
        let backend = Arc::new(TestBackend::new().with_position(position(6004, 0.0, 0.0, 1.0))); // volume mismatch
        let (report_tx, _report_rx) = broadcast::channel(16);

        run_pass(&manager, &backend, &report_tx, AutoAckPolicy::Never).await;
        assert_eq!(
            manager.get_order(&cid).unwrap().state,
            OrderState::Mismatched,
            "Never policy must leave the mismatch unacked"
        );
    }

    #[tokio::test]
    async fn config_defaults() {
        let config = ReconcilerConfig::default();
        assert_eq!(config.interval, std::time::Duration::from_secs(30));
        assert!(config.on_unknown);
        assert!(config.on_event_gap);
        assert_eq!(config.auto_acknowledge, AutoAckPolicy::Never);
    }

    #[test]
    fn safety_policy_default_keeps_unknown_unresolved() {
        // Sanity check that the default policy requires repeated absence, so an
        // Unknown order stays unresolved (blocking) until reconciled.
        let policy = SafetyPolicy::default();
        assert_eq!(policy.min_absent_observations, 3);
        assert_eq!(policy.min_absent_secs, 30);
    }
}

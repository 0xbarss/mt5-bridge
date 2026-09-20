//! High-level Order Manager and Reconciliation Engine.
//!
//! Provides:
//! - Full order lifecycle tracking via [`OrderManager`].
//! - Explicit order idempotency guarantees via client order IDs.
//! - Post-disconnect and startup broker reconciliation via [`ReconciliationReport`].
//! - Strict strategy ownership enforcement via magic numbers.
//! - First-class handling of uncertain execution (`OrderState::Unknown`).

use crate::client::Mt5Client;
use crate::error::{Mt5Error, Result};
use crate::types::{
    OrderRequest, OrderState, Position, TrackedOrder, TradeResult, WorkingOrder,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

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
    /// Encountered communication failure or unknown execution state.
    Degraded,
    /// Bridge shut down.
    Stopped,
}

/// Comprehensive report produced by a reconciliation cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationReport {
    /// All active open positions matching strategy magic number.
    pub positions: Vec<Position>,
    /// All working pending orders matching strategy magic number.
    pub pending_orders: Vec<WorkingOrder>,
    /// Orders that were resolved from `Unknown` or `Submitting` to active/filled.
    pub reconciled_orders: Vec<TrackedOrder>,
    /// Tracked orders that were `Unknown` and confirmed absent from MT5.
    pub absent_orders: Vec<TrackedOrder>,
    /// Open positions found in MT5 that were not tracked by local memory.
    pub foreign_positions: Vec<Position>,
    /// UTC timestamp of the reconciliation pass.
    pub timestamp: i64,
}

impl ReconciliationReport {
    /// Returns true if all open positions and working orders match local state with zero unknown orders.
    pub fn is_clean(&self) -> bool {
        self.absent_orders.is_empty() && self.foreign_positions.is_empty()
    }
}

/// High-level execution and order lifecycle manager.
///
/// Wraps [`Mt5Client`] with:
/// - Order idempotency: ensures repeated submits with the same `client_order_id` do not create duplicate exposure.
/// - Explicit order state machine: tracks volume and states (`Submitting`, `Filled`, `Partial`, `Unknown`).
/// - Post-failure reconciliation: queries live broker state and resolves ambiguous execution outcomes.
/// - Strategy ownership: enforces strategy magic number on all close, modify, and query actions.
pub struct OrderManager {
    strategy_magic: u64,
    lifecycle: LifecycleState,
    orders: HashMap<String, TrackedOrder>,
}

impl OrderManager {
    /// Create a new `OrderManager` bound to a specific strategy magic number.
    pub fn new(strategy_magic: u64) -> Self {
        Self {
            strategy_magic,
            lifecycle: LifecycleState::Starting,
            orders: HashMap::new(),
        }
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

    /// Retrieve all tracked orders.
    pub fn tracked_orders(&self) -> Vec<&TrackedOrder> {
        self.orders.values().collect()
    }

    /// Find a tracked order by its unique client order ID.
    pub fn get_order(&self, client_order_id: &str) -> Option<&TrackedOrder> {
        self.orders.get(client_order_id)
    }

    /// Generate a deterministic, collision-resistant client order ID if not provided.
    pub fn generate_client_order_id() -> String {
        let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        format!("ord-{:x}", ts)
    }

    /// Register a tracked order directly into the manager (e.g. during restart recovery from persistent storage).
    pub fn track_order(&mut self, order: TrackedOrder) {
        self.orders.insert(order.client_order_id.clone(), order);
    }

    /// Restore multiple tracked orders into the manager (e.g. during restart recovery from persistent storage).
    pub fn restore_orders(&mut self, orders: impl IntoIterator<Item = TrackedOrder>) {
        for order in orders {
            self.track_order(order);
        }
    }

    /// Export all tracked orders for persistent storage or state serialization.
    pub fn export_orders(&self) -> Vec<TrackedOrder> {
        self.orders.values().cloned().collect()
    }

    /// Validates strategy magic ownership and checks the idempotency guard for an order request.
    ///
    /// Returns:
    /// - `Ok(Some(tracked))` if the order was already executed/tracked (idempotent replay).
    /// - `Ok(None)` if the order is new and must be transmitted to the broker.
    /// - `Err(Mt5Error::OwnershipMismatch)` if the order's magic number violates strategy ownership.
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
                if existing.is_terminal() || existing.is_active() {
                    return Ok(Some(existing.clone()));
                }
            }
        }

        Ok(None)
    }

    /// Submit a trading order with idempotency protection and lifecycle state tracking.
    ///
    /// If an order with the same `client_order_id` is already tracked and active or completed,
    /// this function will return the existing order without sending a duplicate trade to the broker.
    pub fn submit_order(
        &mut self,
        client: &Mt5Client,
        mut req: OrderRequest,
    ) -> Result<TrackedOrder> {
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

        // Initialize tracked order
        let mut tracked = TrackedOrder::new(&req, &cid);
        tracked.mark_submitting();
        self.orders.insert(cid.clone(), tracked.clone());

        // Transmit to MT5 bridge
        match client.order_send(&req) {
            Ok(trade_res) => {
                tracked.update_from_trade_result(&trade_res);
                self.orders.insert(cid.clone(), tracked.clone());
                info!(
                    client_order_id = %cid,
                    state = ?tracked.state,
                    order = tracked.order_ticket,
                    deal = tracked.deal_ticket,
                    price = tracked.average_price,
                    "Order executed and tracked successfully"
                );
                Ok(tracked)
            }
            Err(Mt5Error::UnknownExecutionState { description, .. }) => {
                // Ambiguous network outcome: response lost after submission!
                // Mark state as Unknown and set lifecycle to Degraded.
                tracked.mark_unknown(&description);
                self.orders.insert(cid.clone(), tracked.clone());
                self.lifecycle = LifecycleState::Degraded;
                warn!(
                    client_order_id = %cid,
                    "Order execution outcome is UNKNOWN! Reconcile against broker before retrying."
                );
                Err(Mt5Error::UnknownExecutionState {
                    symbol: req.symbol,
                    client_order_id: Some(cid),
                    description,
                })
            }
            Err(e) => {
                tracked.state = OrderState::Rejected;
                tracked.error_message = Some(e.to_string());
                tracked.updated_at = chrono::Utc::now().timestamp();
                self.orders.insert(cid, tracked.clone());
                Err(e)
            }
        }
    }

    /// Close an existing position owned by this strategy.
    pub fn close_position(&self, client: &Mt5Client, ticket: u64) -> Result<TradeResult> {
        client.order_close_with_magic(ticket, self.strategy_magic)
    }

    /// Modify Stop Loss / Take Profit for a position or pending order owned by this strategy.
    pub fn modify_order(
        &self,
        client: &Mt5Client,
        ticket: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        client.order_modify_with_magic(ticket, self.strategy_magic, stop_loss, take_profit)
    }

    /// Reconcile local tracked orders against live MT5 positions and working orders.
    ///
    /// Resolves `OrderState::Unknown` and `OrderState::Submitting` orders:
    /// - If an unknown order is found on the broker, its state is moved to `Reconciled` or `Accepted`.
    /// - If an unknown order is definitively absent from the broker, it is resolved to `Rejected`.
    pub fn reconcile(&mut self, client: &Mt5Client) -> Result<ReconciliationReport> {
        self.lifecycle = LifecycleState::Reconciling;

        let live_positions = client.positions_filtered(Some(self.strategy_magic), None)?;
        let live_orders = client.pending_orders_filtered(Some(self.strategy_magic), None)?;

        Ok(self.reconcile_with_snapshot(live_positions, live_orders))
    }

    /// Reconcile local tracked orders against an explicit snapshot of broker positions and working orders.
    ///
    /// This method is independent of live network I/O, allowing deterministic unit testing,
    /// audit replays, and post-restart state reconstruction.
    pub fn reconcile_with_snapshot(
        &mut self,
        live_positions: Vec<Position>,
        live_orders: Vec<WorkingOrder>,
    ) -> ReconciliationReport {
        self.lifecycle = LifecycleState::Reconciling;

        let mut reconciled_orders = Vec::new();
        let mut absent_orders = Vec::new();
        let mut matched_position_tickets = std::collections::HashSet::new();

        for tracked in self.orders.values_mut() {
            if tracked.state == OrderState::Unknown || tracked.state == OrderState::Submitting {
                // Check if found in live positions
                let pos_match = live_positions.iter().find(|p| {
                    (tracked.position_ticket > 0 && p.ticket == tracked.position_ticket)
                        || (p.comment.contains(&tracked.client_order_id))
                });

                if let Some(pos) = pos_match {
                    matched_position_tickets.insert(pos.ticket);
                    tracked.reconcile_with_position(pos);
                    reconciled_orders.push(tracked.clone());
                    info!(
                        client_order_id = %tracked.client_order_id,
                        ticket = pos.ticket,
                        "Reconciled unknown order against live position"
                    );
                    continue;
                }

                // Check if found in live working pending orders
                let ord_match = live_orders.iter().find(|o| {
                    (tracked.order_ticket > 0 && o.ticket == tracked.order_ticket)
                        || (o.comment.contains(&tracked.client_order_id))
                });

                if let Some(ord) = ord_match {
                    tracked.reconcile_with_working_order(ord);
                    reconciled_orders.push(tracked.clone());
                    info!(
                        client_order_id = %tracked.client_order_id,
                        ticket = ord.ticket,
                        "Reconciled unknown order against live pending order"
                    );
                    continue;
                }

                // Definitively absent from both live positions and pending orders
                tracked.state = OrderState::Rejected;
                tracked.error_message = Some(
                    "Reconciliation confirmed order was not placed with broker".to_string(),
                );
                tracked.updated_at = chrono::Utc::now().timestamp();
                absent_orders.push(tracked.clone());
                warn!(
                    client_order_id = %tracked.client_order_id,
                    "Unknown order confirmed absent from broker during reconciliation"
                );
            }
        }

        // Identify live positions not matched to any locally tracked order
        let foreign_positions: Vec<Position> = live_positions
            .iter()
            .filter(|p| !matched_position_tickets.contains(&p.ticket))
            .filter(|p| {
                !self
                    .orders
                    .values()
                    .any(|o| o.position_ticket == p.ticket)
            })
            .cloned()
            .collect();

        self.lifecycle = LifecycleState::Ready;

        ReconciliationReport {
            positions: live_positions,
            pending_orders: live_orders,
            reconciled_orders,
            absent_orders,
            foreign_positions,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}

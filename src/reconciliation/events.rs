//! Order lifecycle events and the [`OrderObserver`] trait.
//!
//! [`OrderManager`](crate::reconciliation::OrderManager) emits one of these events on every
//! tracked state transition. Observers receive them **synchronously**, so the reconciliation
//! core stays free of Tokio: an observer that needs fan-out forwards to a
//! `tokio::sync::broadcast::Sender`, as [`Mt5Gateway`](crate::gateway::Mt5Gateway) does.

use crate::position_book::DealReason;
use crate::reconciliation::state::ReconciliationReport;
use crate::types::{MismatchKind, OrderRequest};

/// Notification emitted by [`OrderManager`](crate::reconciliation::OrderManager) on each
/// tracked order state transition.
///
/// All fields are owned and `Clone`, so observers can log, journal, or broadcast events
/// without borrowing tracked state.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum OrderEvent {
    /// The order passed idempotency and magic checks and was recorded as `Submitting`,
    /// about to be transmitted to MT5.
    Submitting {
        client_order_id: String,
        request: OrderRequest,
    },
    /// The order was accepted by the broker and is working in the order book (a pending
    /// order, or a market order acknowledged before any fill).
    Accepted {
        client_order_id: String,
        order_ticket: u64,
    },
    /// Part of the order filled; the remainder is still working.
    PartialFill {
        client_order_id: String,
        filled: f64,
        remaining: f64,
    },
    /// The order filled completely.
    Filled {
        client_order_id: String,
        deal_ticket: u64,
        price: f64,
    },
    /// The order was rejected by the bridge or broker.
    Rejected {
        client_order_id: String,
        reason: String,
    },
    /// The execution outcome is ambiguous (timeout, lost response) and must be reconciled
    /// against broker state before any retry.
    Unknown {
        client_order_id: String,
        description: String,
    },
    /// Reconciliation confirmed the order matches broker state exactly.
    Reconciled {
        client_order_id: String,
        report: ReconciliationReport,
    },
    /// Reconciliation found the order on the broker, but one or more attributes differ
    /// from the original request.
    Mismatched {
        client_order_id: String,
        mismatches: Vec<MismatchKind>,
    },
    /// A tracked position or order was closed.
    Closed {
        client_order_id: String,
        reason: DealReason,
    },
}

/// Observes [`OrderEvent`] notifications from an [`OrderManager`](crate::reconciliation::OrderManager).
///
/// Implementations must not block: [`OrderManager`](crate::reconciliation::OrderManager)
/// calls `on_event` synchronously while a mutation is in progress. For delivery to many
/// consumers, forward to a `tokio::sync::broadcast::Sender` and let each consumer subscribe.
pub trait OrderObserver: Send + Sync {
    /// Handle a lifecycle event emitted during a state transition.
    fn on_event(&self, event: &OrderEvent);
}

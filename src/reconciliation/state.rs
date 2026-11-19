use crate::types::{Deal, Position, TrackedOrder, WorkingOrder};
use serde::{Deserialize, Serialize};

/// Slack (seconds) added on both sides of the deal-history window fetched by
/// [`OrderManager::reconcile`](crate::reconciliation::OrderManager::reconcile), to tolerate clock skew between this host and the broker.
pub(crate) const DEAL_WINDOW_SLACK_SECS: i64 = 900;

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

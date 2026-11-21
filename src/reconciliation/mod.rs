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

pub mod classification;
pub mod manager;
pub mod shared;
pub mod state;

#[cfg(feature = "async")]
pub(crate) use classification::classify_send_error;
pub use manager::OrderManager;
pub use shared::SharedOrderManager;
pub use state::{
    BrokerSnapshot, LifecycleState, ReconciliationReport, SafetyPolicy, UnknownBlockScope,
};

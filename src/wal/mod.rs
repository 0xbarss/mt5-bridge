//! Durable write-ahead log (WAL) order store.
//!
//! Provides high-throughput, low-latency append-only order journaling.
//! Pre-send intents (`OrderState::Submitting`) are written as small binary-framed
//! entries with synchronous `fsync`, while subsequent state transitions (e.g. fills,
//! modifications, closures) are appended incrementally without rewriting the entire state.
//!
//! Periodic atomic snapshots (`orders.snap`) and pruning keep WAL files compact and bounded.

pub mod format;
pub mod migration;
pub mod store;

pub use format::{crc32, WalConfig, CRC_POLY_IEEE, CRC_POLY_V2};
#[cfg(feature = "json-journal")]
pub use migration::{migrate_json_to_wal, wal_to_json};
pub use store::WalOrderStore;

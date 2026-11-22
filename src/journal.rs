//! Durable persistence of tracked orders.
//!
//! Crash recovery only works if the record that an order **may have been sent** reaches
//! stable storage *before* the request leaves the process. [`OrderManager`](crate::OrderManager)
//! therefore journals every state change through an [`OrderStore`] and refuses to transmit an
//! order whose write-ahead record could not be saved.
//!
//! * [`OrderStore`] — the abstraction. Implement it over SQLite, a database, etc. if you
//!   prefer; the manager only needs "save everything atomically" and "load everything".
//! * [`JsonFileStore`] — a reference implementation (feature `serde_json`, enabled by
//!   default): write to a temp file in the same directory, `fsync` it, atomically `rename`
//!   over the target, then `fsync` the directory (on Unix). A crash at any point leaves
//!   either the complete old file or the complete new one, never a torn mix.
//!
//! **Single writer:** a store must be owned by exactly one process at a time. Two processes
//! journaling to the same file will silently overwrite each other.

use crate::error::Result;
use crate::types::{TrackedOrder, TradeRule};

/// Durable storage backend for [`TrackedOrder`] state.
pub trait OrderStore: Send {
    /// Atomically replace the stored state with `orders`. On `Ok`, the data must have reached
    /// stable storage. On `Err`, the previously stored state must be left intact.
    fn save(&self, orders: &[TrackedOrder]) -> Result<()>;

    /// Load all stored orders. A store that has never been written returns an empty list.
    /// Corrupt or unreadable state **must** be an error, never an empty list: silently
    /// starting empty would forget in-flight orders.
    fn load(&self) -> Result<Vec<TrackedOrder>>;

    /// Returns `true` if this store supports incremental append-only records (e.g. WAL).
    /// Stores returning `false` (e.g. [`JsonFileStore`]) rewrite all state on each snapshot.
    fn is_incremental(&self) -> bool {
        false
    }

    /// Journal a pre-send order intent (write-ahead record).
    /// The record must reach stable storage before this call returns `Ok`.
    fn record_intent(&self, order: &TrackedOrder) -> Result<()> {
        let _ = order;
        Ok(())
    }

    /// Journal an incremental order state update (fill, modify, close, reject).
    fn record_update(&self, order: &TrackedOrder) -> Result<()> {
        let _ = order;
        Ok(())
    }

    /// Journal a trade-management rule (`crate::types::TradeRule`).
    ///
    /// The default implementation is a no-op: stores that do not support rule
    /// persistence (e.g. `JsonFileStore`) silently accept and discard. [`crate::WalOrderStore`]
    /// implements this with a dedicated `REC_TRADE_RULE` frame so rules survive restart.
    fn record_trade_rule(&self, _symbol: &str, _rule: &TradeRule) -> Result<()> {
        Ok(())
    }

    /// Load trade rules for `symbol` from durable storage.
    ///
    /// The default implementation returns an empty vector. [`crate::WalOrderStore`] replays
    /// `REC_TRADE_RULE` frames from the WAL.
    fn load_trade_rules(&self, _symbol: &str) -> Result<Vec<TradeRule>> {
        Ok(Vec::new())
    }

    /// Prune settled / terminal orders from durable storage.
    fn prune(&self, keep_client_order_ids: &[String]) -> Result<()> {
        let _ = keep_client_order_ids;
        Ok(())
    }

    /// Flush any pending buffered records to disk.
    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(feature = "json-journal")]
pub use json_file::JsonFileStore;

#[cfg(feature = "json-journal")]
mod json_file {
    use super::OrderStore;
    use crate::error::{Mt5Error, Result};
    use crate::types::TrackedOrder;
    use serde::{Deserialize, Serialize};
    use std::fs::{self, File};
    use std::io::{ErrorKind, Write};
    use std::path::{Path, PathBuf};

    const FORMAT: &str = "mt5-bridge/orders";
    const VERSION: u32 = 1;

    #[derive(Serialize, Deserialize)]
    struct Envelope {
        format: String,
        version: u32,
        #[serde(default)]
        generation: u64,
        #[serde(default)]
        checksum: String,
        saved_at: i64,
        orders: Vec<TrackedOrder>,
    }

    fn compute_checksum(orders: &[TrackedOrder]) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let serialized = serde_json::to_vec(orders).unwrap_or_default();
        for &b in &serialized {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{h:016x}")
    }

    /// Atomic, fsync'd JSON file implementation of [`OrderStore`].
    ///
    /// The whole file is rewritten on every save (O(number of tracked orders)); that is
    /// intentional — it keeps every on-disk state self-consistent and trivially recoverable.
    /// Long-running strategies should periodically prune terminal orders from the manager.
    #[derive(Debug, Clone)]
    pub struct JsonFileStore {
        path: PathBuf,
        generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    }

    fn io_err(what: &str, path: &Path, e: std::io::Error) -> Mt5Error {
        Mt5Error::PersistenceError(format!("{what} '{}': {e}", path.display()))
    }

    impl JsonFileStore {
        /// Create a store backed by `path`. The parent directory must exist.
        pub fn new(path: impl Into<PathBuf>) -> Self {
            Self {
                path: path.into(),
                generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            }
        }

        /// Path of the backing file.
        pub fn path(&self) -> &Path {
            &self.path
        }

        fn tmp_path(&self) -> PathBuf {
            let mut name = self
                .path
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default();
            name.push(".tmp");
            self.path.with_file_name(name)
        }

        fn bak_path(&self) -> PathBuf {
            let mut name = self
                .path
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default();
            name.push(".bak");
            self.path.with_file_name(name)
        }

        fn parse_and_verify(&self, bytes: &[u8], path: &Path) -> Result<Vec<TrackedOrder>> {
            let envelope: Envelope = serde_json::from_slice(bytes).map_err(|e| {
                Mt5Error::PersistenceError(format!(
                    "'{}' is corrupt or not an order journal: {e}. Refusing to start with an empty order book; inspect or restore the file.",
                    path.display()
                ))
            })?;
            if envelope.format != FORMAT || envelope.version > VERSION {
                return Err(Mt5Error::PersistenceError(format!(
                    "'{}' has unsupported format '{}' v{}",
                    path.display(),
                    envelope.format,
                    envelope.version
                )));
            }
            if !envelope.checksum.is_empty() {
                let expected = compute_checksum(&envelope.orders);
                if envelope.checksum != expected {
                    return Err(Mt5Error::PersistenceError(format!(
                        "'{}' checksum mismatch: expected {expected}, recorded {}. Possible data corruption.",
                        path.display(),
                        envelope.checksum
                    )));
                }
            }
            self.generation
                .store(envelope.generation, std::sync::atomic::Ordering::SeqCst);
            Ok(envelope.orders)
        }
    }

    impl OrderStore for JsonFileStore {
        fn save(&self, orders: &[TrackedOrder]) -> Result<()> {
            let gen = self
                .generation
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            let checksum = compute_checksum(orders);
            let envelope = Envelope {
                format: FORMAT.to_string(),
                version: VERSION,
                generation: gen,
                checksum,
                saved_at: chrono::Utc::now().timestamp(),
                orders: orders.to_vec(),
            };
            let bytes = serde_json::to_vec_pretty(&envelope)
                .map_err(|e| Mt5Error::PersistenceError(format!("serialize failed: {e}")))?;

            let tmp = self.tmp_path();
            {
                let mut f = File::create(&tmp).map_err(|e| io_err("create", &tmp, e))?;
                f.write_all(&bytes).map_err(|e| io_err("write", &tmp, e))?;
                // Data must be on stable storage *before* the rename makes it visible.
                f.sync_all().map_err(|e| io_err("fsync", &tmp, e))?;
            }

            if self.path.exists() {
                let _ = fs::copy(&self.path, self.bak_path());
            }

            fs::rename(&tmp, &self.path).map_err(|e| io_err("rename onto", &self.path, e))?;

            // Make the rename itself durable. (Directories cannot be opened for fsync on Windows;
            // NTFS journals the rename metadata itself.)
            #[cfg(unix)]
            {
                let dir = match self.path.parent() {
                    Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                    _ => PathBuf::from("."),
                };
                File::open(&dir)
                    .and_then(|d| d.sync_all())
                    .map_err(|e| io_err("fsync directory", &dir, e))?;
            }
            Ok(())
        }

        fn load(&self) -> Result<Vec<TrackedOrder>> {
            let bytes = match fs::read(&self.path) {
                Ok(b) => b,
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    let bak = self.bak_path();
                    if bak.exists() {
                        if let Ok(b) = fs::read(&bak) {
                            tracing::warn!(
                                "Primary journal missing; recovered from backup '{}'",
                                bak.display()
                            );
                            return self.parse_and_verify(&b, &bak);
                        }
                    }
                    return Ok(Vec::new());
                }
                Err(e) => return Err(io_err("read", &self.path, e)),
            };

            match self.parse_and_verify(&bytes, &self.path) {
                Ok(orders) => Ok(orders),
                Err(primary_err) => {
                    let bak = self.bak_path();
                    if bak.exists() {
                        if let Ok(b) = fs::read(&bak) {
                            if let Ok(orders) = self.parse_and_verify(&b, &bak) {
                                tracing::warn!(
                                    "Primary journal corrupt; restored from backup '{}'",
                                    bak.display()
                                );
                                return Ok(orders);
                            }
                        }
                    }
                    Err(primary_err)
                }
            }
        }
    }
}

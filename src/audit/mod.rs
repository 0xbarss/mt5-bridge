//! Structured audit log with hash chaining.
//!
//! Append-only, tamper-evident journal of order-lifecycle and configuration decisions.
//! Each line is a JSON object containing the event payload plus a SHA-256 hash chain:
//! `hash = SHA256(prev_hash || canonical_json(event))`.
//!
//! The first entry uses an all-zeros genesis hash as `prev_hash`.
//! [`AuditLog::verify`] replays the file and detects tampering (modified hash or
//! payload) and truncation (a truncated or unparseable final line).
//!
//! [`AuditLog`] implements [`crate::reconciliation::events::OrderObserver`], so it can be
//! attached to an [`OrderManager`](crate::reconciliation::OrderManager) via
//! [`with_observer`](crate::reconciliation::OrderManager::with_observer) to audit every
//! lifecycle decision the manager makes.

use crate::reconciliation::events::{OrderEvent, OrderObserver};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// All-zeros SHA-256 digest used as the `prev_hash` of the very first entry.
const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One auditable decision: order submission, risk rejection, send outcome,
/// reconciliation, acknowledgement, or configuration change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuditEvent {
    /// An order was submitted to the MT5 bridge.
    Submit {
        client_order_id: String,
        symbol: String,
        side: String,
        volume: f64,
    },
    /// A pre-trade risk rule rejected an order before it reached the backend.
    RiskReject {
        client_order_id: String,
        rule: String,
        detail: String,
    },
    /// The broker accepted, filled, partially filled, rejected, or could not resolve
    /// an order.
    SendOutcome {
        client_order_id: String,
        outcome: String,
        order_ticket: Option<u64>,
        deal_ticket: Option<u64>,
        price: Option<f64>,
    },
    /// Reconciliation resolved an order against live broker state.
    Reconcile {
        client_order_id: String,
        status: String,
        mismatches: Vec<String>,
    },
    /// An operator acknowledged a mismatched order.
    Acknowledge { client_order_id: String, by: String },
    /// A configuration value changed.
    ConfigChange {
        key: String,
        old: Option<String>,
        new: Option<String>,
    },
}

/// One chained entry written to the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuditEntry {
    hash: String,
    prev_hash: String,
    event: AuditEvent,
}

/// Error reported by [`AuditLog::verify`].
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// The log file could not be read.
    #[error("I/O error reading audit log: {0}")]
    Io(#[from] std::io::Error),
    /// A line is truncated or unparseable JSON.
    #[error("line {line} is truncated or unparseable")]
    Truncated { line: usize },
    /// The chain is not continuous: `prev_hash` of entry `n` != hash of entry `n-1`.
    #[error("chain break at line {line}: expected prev_hash {expected}, got {got}")]
    ChainBreak {
        line: usize,
        expected: String,
        got: String,
    },
    /// An entry's hash does not match `SHA256(prev_hash || canonical_json(event))`.
    #[error("tampered entry at line {line}: expected hash {expected}, got {got}")]
    Tampered {
        line: usize,
        expected: String,
        got: String,
    },
}

/// Append-only, tamper-evident audit log backed by a JSON Lines file.
///
/// Each call to [`append`](Self::append) appends one JSON line whose `hash` is
/// `SHA256(prev_hash || canonical_json(event))`, chaining to the previous entry.
/// The chain tip is recovered from the file on first use so the log survives
/// process restarts.
///
/// Use [`verify`](Self::verify) to replay the file and detect tampering or truncation.
pub struct AuditLog {
    path: PathBuf,
    tip: Mutex<Option<String>>,
}

impl AuditLog {
    /// Open (or create) the audit log at `path`.
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            tip: Mutex::new(None),
        }
    }

    /// Append `event` to the log, returning the new entry's hash.
    pub fn append(&self, event: &AuditEvent) -> Result<String, std::io::Error> {
        let event_json = serde_json::to_string(event).expect("audit event serializes");

        let prev = {
            let mut guard = self.tip.lock().unwrap();
            if guard.is_none() {
                *guard = self.read_tip()?;
            }
            guard.clone().unwrap_or_else(|| GENESIS_HASH.to_string())
        };

        let hash = hash_chain(&prev, &event_json);
        let entry = AuditEntry {
            hash: hash.clone(),
            prev_hash: prev,
            event: event.clone(),
        };
        let line = serde_json::to_string(&entry).expect("audit entry serializes");

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;

        *self.tip.lock().unwrap() = Some(hash.clone());
        Ok(hash)
    }

    /// Verify the hash chain of the log file at `path`.
    ///
    /// Returns an error if any line is unparseable (truncation or corruption), if any
    /// entry's hash does not match `SHA256(prev_hash || canonical_json(event))`, or if
    /// the chain is not continuous (`prev_hash` of entry `n` != hash of entry `n-1`).
    pub fn verify<P: AsRef<Path>>(path: P) -> Result<(), VerifyError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(VerifyError::Io)?;
        let reader = BufReader::new(file);

        let mut prev_hash = GENESIS_HASH.to_string();
        for (line_no, line) in reader.lines().enumerate() {
            let line_no = line_no + 1;
            let line = line.map_err(VerifyError::Io)?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: AuditEntry = serde_json::from_str(trimmed)
                .map_err(|_| VerifyError::Truncated { line: line_no })?;
            if entry.prev_hash != prev_hash {
                return Err(VerifyError::ChainBreak {
                    line: line_no,
                    expected: prev_hash,
                    got: entry.prev_hash,
                });
            }
            let event_json = serde_json::to_string(&entry.event)
                .map_err(|_| VerifyError::Truncated { line: line_no })?;
            let expected_hash = hash_chain(&entry.prev_hash, &event_json);
            if entry.hash != expected_hash {
                return Err(VerifyError::Tampered {
                    line: line_no,
                    expected: expected_hash,
                    got: entry.hash,
                });
            }
            prev_hash = entry.hash;
        }
        Ok(())
    }

    /// Recover the chain tip (last entry's hash) from the existing file, if any.
    fn read_tip(&self) -> Result<Option<String>, std::io::Error> {
        if !self.path.exists() {
            return Ok(None);
        }
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut last_hash: Option<String> = None;
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<AuditEntry>(trimmed) {
                last_hash = Some(entry.hash);
            }
        }
        Ok(last_hash)
    }

    /// Record an explicit configuration change (not driven by an order lifecycle event).
    pub fn record_config_change(
        &self,
        key: &str,
        old: Option<&str>,
        new: Option<&str>,
    ) -> Result<String, std::io::Error> {
        self.append(&AuditEvent::ConfigChange {
            key: key.to_string(),
            old: old.map(|s| s.to_string()),
            new: new.map(|s| s.to_string()),
        })
    }

    /// Record an operator acknowledgement of a mismatched order.
    pub fn record_acknowledge(
        &self,
        client_order_id: &str,
        by: &str,
    ) -> Result<String, std::io::Error> {
        self.append(&AuditEvent::Acknowledge {
            client_order_id: client_order_id.to_string(),
            by: by.to_string(),
        })
    }
}

impl OrderObserver for AuditLog {
    fn on_event(&self, event: &OrderEvent) {
        let audit_event = AuditEvent::from(event);
        if let Err(e) = self.append(&audit_event) {
            tracing::warn!(error = %e, "failed to append order lifecycle event to audit log");
        }
    }
}

impl From<&OrderEvent> for AuditEvent {
    fn from(event: &OrderEvent) -> Self {
        match event {
            OrderEvent::Submitting {
                client_order_id,
                request,
            } => AuditEvent::Submit {
                client_order_id: client_order_id.clone(),
                symbol: request.symbol.clone(),
                side: format!("{:?}", request.order_type),
                volume: request.volume,
            },
            OrderEvent::Accepted {
                client_order_id,
                order_ticket,
            } => AuditEvent::SendOutcome {
                client_order_id: client_order_id.clone(),
                outcome: "accepted".to_string(),
                order_ticket: Some(*order_ticket),
                deal_ticket: None,
                price: None,
            },
            OrderEvent::PartialFill {
                client_order_id,
                filled: _,
                remaining: _,
            } => AuditEvent::SendOutcome {
                client_order_id: client_order_id.clone(),
                outcome: "partial_fill".to_string(),
                order_ticket: None,
                deal_ticket: None,
                price: None,
            },
            OrderEvent::Filled {
                client_order_id,
                deal_ticket,
                price,
            } => AuditEvent::SendOutcome {
                client_order_id: client_order_id.clone(),
                outcome: "filled".to_string(),
                order_ticket: None,
                deal_ticket: Some(*deal_ticket),
                price: Some(*price),
            },
            OrderEvent::Rejected {
                client_order_id,
                reason: _,
            } => AuditEvent::SendOutcome {
                client_order_id: client_order_id.clone(),
                outcome: "rejected".to_string(),
                order_ticket: None,
                deal_ticket: None,
                price: None,
            },
            OrderEvent::Unknown {
                client_order_id,
                description: _,
            } => AuditEvent::SendOutcome {
                client_order_id: client_order_id.clone(),
                outcome: "unknown".to_string(),
                order_ticket: None,
                deal_ticket: None,
                price: None,
            },
            OrderEvent::Reconciled {
                client_order_id,
                report: _,
            } => AuditEvent::Reconcile {
                client_order_id: client_order_id.clone(),
                status: "reconciled".to_string(),
                mismatches: Vec::new(),
            },
            OrderEvent::Mismatched {
                client_order_id,
                mismatches,
            } => AuditEvent::Reconcile {
                client_order_id: client_order_id.clone(),
                status: "mismatched".to_string(),
                mismatches: mismatches.iter().map(|m| format!("{m:?}")).collect(),
            },
            OrderEvent::Closed {
                client_order_id,
                reason: _,
            } => AuditEvent::SendOutcome {
                client_order_id: client_order_id.clone(),
                outcome: "closed".to_string(),
                order_ticket: None,
                deal_ticket: None,
                price: None,
            },
        }
    }
}

/// Compute `SHA256(prev_hash || canonical_json(event))` as a lowercase hex string.
fn hash_chain(prev_hash: &str, event_json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(event_json.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("audit_test_{n}.jsonl"))
    }

    #[test]
    fn append_and_verify_chain() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        let log = AuditLog::new(&path);

        let h1 = log
            .append(&AuditEvent::Submit {
                client_order_id: "cid-1".into(),
                symbol: "EURUSD".into(),
                side: "Buy".into(),
                volume: 1.0,
            })
            .unwrap();
        assert!(!h1.is_empty());
        let h2 = log
            .append(&AuditEvent::SendOutcome {
                client_order_id: "cid-1".into(),
                outcome: "filled".into(),
                order_ticket: None,
                deal_ticket: Some(42),
                price: Some(1.085),
            })
            .unwrap();
        assert_ne!(h1, h2);

        AuditLog::verify(&path).unwrap();
    }

    #[test]
    fn verify_detects_tampering() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        let log = AuditLog::new(&path);
        log.append(&AuditEvent::Submit {
            client_order_id: "cid-1".into(),
            symbol: "EURUSD".into(),
            side: "Buy".into(),
            volume: 1.0,
        })
        .unwrap();
        log.append(&AuditEvent::SendOutcome {
            client_order_id: "cid-1".into(),
            outcome: "filled".into(),
            order_ticket: None,
            deal_ticket: Some(42),
            price: Some(1.085),
        })
        .unwrap();

        // Tamper with the first line's payload (change the volume).
        let mut contents = std::fs::read_to_string(&path).unwrap();
        let first_newline = contents.find('\n').unwrap();
        let mut first_line = contents[..first_newline].to_string();
        first_line = first_line.replace(r#""volume":1.0"#, r#""volume":1.9"#);
        contents.replace_range(..first_newline, &first_line);
        std::fs::write(&path, contents).unwrap();

        let err = AuditLog::verify(&path).unwrap_err();
        matches!(err, VerifyError::Tampered { .. });
    }

    #[test]
    fn verify_detects_truncation() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        let log = AuditLog::new(&path);
        log.append(&AuditEvent::Submit {
            client_order_id: "cid-1".into(),
            symbol: "EURUSD".into(),
            side: "Buy".into(),
            volume: 1.0,
        })
        .unwrap();

        // Truncate the file mid-line.
        let mut contents = std::fs::read_to_string(&path).unwrap();
        contents.truncate(contents.len() / 2);
        std::fs::write(&path, contents).unwrap();

        let err = AuditLog::verify(&path).unwrap_err();
        matches!(err, VerifyError::Truncated { .. });
    }

    #[test]
    fn verify_detects_chain_break() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        let log = AuditLog::new(&path);
        log.append(&AuditEvent::Submit {
            client_order_id: "cid-1".into(),
            symbol: "EURUSD".into(),
            side: "Buy".into(),
            volume: 1.0,
        })
        .unwrap();
        let _ = log
            .append(&AuditEvent::SendOutcome {
                client_order_id: "cid-1".into(),
                outcome: "filled".into(),
                order_ticket: None,
                deal_ticket: Some(42),
                price: Some(1.085),
            })
            .unwrap();

        // Swap the two lines so prev_hash no longer matches the previous hash.
        let contents = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<&str> = contents.lines().collect();
        lines.swap(0, 1);
        std::fs::write(&path, lines.join("\n")).unwrap();

        let err = AuditLog::verify(&path).unwrap_err();
        matches!(err, VerifyError::ChainBreak { .. });
    }

    #[test]
    fn genesis_hash_seeds_first_entry() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        let log = AuditLog::new(&path);
        let hash = log
            .append(&AuditEvent::Submit {
                client_order_id: "cid-1".into(),
                symbol: "EURUSD".into(),
                side: "Buy".into(),
                volume: 1.0,
            })
            .unwrap();
        let entry_json = std::fs::read_to_string(&path).unwrap();
        let entry: AuditEntry = serde_json::from_str(entry_json.trim()).unwrap();
        assert_eq!(entry.prev_hash, GENESIS_HASH);
        assert_eq!(entry.hash, hash);
        assert_eq!(
            hash,
            hash_chain(GENESIS_HASH, &serde_json::to_string(&entry.event).unwrap())
        );
    }

    #[test]
    fn order_event_conversion() {
        use crate::reconciliation::events::OrderEvent;
        use crate::types::OrderRequest;

        let ev = OrderEvent::Submitting {
            client_order_id: "cid-1".into(),
            request: OrderRequest::buy("EURUSD", 1.5),
        };
        let audit: AuditEvent = AuditEvent::from(&ev);
        match audit {
            AuditEvent::Submit {
                client_order_id,
                symbol,
                side,
                volume,
            } => {
                assert_eq!(client_order_id, "cid-1");
                assert_eq!(symbol, "EURUSD");
                assert_eq!(side, "Buy");
                assert_eq!(volume, 1.5);
            }
            other => panic!("expected Submit, got {other:?}"),
        }
    }

    #[test]
    fn verify_empty_file_passes() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "").unwrap();
        AuditLog::verify(&path).unwrap();
    }
}

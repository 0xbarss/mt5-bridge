//! Integration tests for the structured audit log with hash chaining (C2).
//!
//! Covers append/verify, tamper detection, and truncation detection through the
//! public [`crate::audit::AuditLog`] API.

use std::fs;
use std::path::PathBuf;

use mt5_bridge::audit::{AuditEvent, AuditLog, VerifyError};

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("audit_integration_{name}.jsonl"));
    let _ = fs::remove_file(&path);
    path
}

fn submit_event() -> AuditEvent {
    AuditEvent::Submit {
        client_order_id: "cid-1".into(),
        symbol: "EURUSD".into(),
        side: "Buy".into(),
        volume: 1.0,
    }
}

fn filled_event() -> AuditEvent {
    AuditEvent::SendOutcome {
        client_order_id: "cid-1".into(),
        outcome: "filled".into(),
        order_ticket: None,
        deal_ticket: Some(42),
        price: Some(1.085),
    }
}

#[test]
fn append_and_verify_chain() {
    let path = temp_path("append_verify");
    let log = AuditLog::new(&path);

    let h1 = log.append(&submit_event()).unwrap();
    assert!(!h1.is_empty());
    let h2 = log.append(&filled_event()).unwrap();
    assert_ne!(h1, h2);

    AuditLog::verify(&path).unwrap();
}

#[test]
fn verify_detects_tampering() {
    let path = temp_path("tamper");
    let log = AuditLog::new(&path);
    log.append(&submit_event()).unwrap();
    log.append(&filled_event()).unwrap();

    let mut contents = fs::read_to_string(&path).unwrap();
    let first_newline = contents.find('\n').unwrap();
    let mut first_line = contents[..first_newline].to_string();
    first_line = first_line.replace(r#""volume":1.0"#, r#""volume":1.9"#);
    contents.replace_range(..first_newline, &first_line);
    fs::write(&path, contents).unwrap();

    let err = AuditLog::verify(&path).unwrap_err();
    assert!(matches!(err, VerifyError::Tampered { .. }));
}

#[test]
fn verify_detects_truncation() {
    let path = temp_path("truncation");
    let log = AuditLog::new(&path);
    log.append(&submit_event()).unwrap();

    let mut contents = fs::read_to_string(&path).unwrap();
    contents.truncate(contents.len() / 2);
    fs::write(&path, contents).unwrap();

    let err = AuditLog::verify(&path).unwrap_err();
    assert!(matches!(err, VerifyError::Truncated { .. }));
}

#[test]
fn verify_detects_chain_break() {
    let path = temp_path("chain_break");
    let log = AuditLog::new(&path);
    log.append(&submit_event()).unwrap();
    log.append(&filled_event()).unwrap();

    let contents = fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = contents.lines().collect();
    lines.swap(0, 1);
    fs::write(&path, lines.join("\n")).unwrap();

    let err = AuditLog::verify(&path).unwrap_err();
    assert!(matches!(err, VerifyError::ChainBreak { .. }));
}

#[test]
fn verify_empty_file_passes() {
    let path = temp_path("empty");
    fs::write(&path, "").unwrap();
    AuditLog::verify(&path).unwrap();
}

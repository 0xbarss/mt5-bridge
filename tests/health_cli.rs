//! Integration tests for the mt5-bridge operational CLI.
//!
//! These tests assert command-line argument parsing and `--help` output using
//! [`assert_cmd`]. The library-side health checks are unit-tested in
//! [`src/health.rs`](crate::health).

use assert_cmd::Command;
use predicates::prelude::*;

fn mt5_bridge() -> Command {
    Command::cargo_bin("mt5-bridge").expect("mt5-bridge binary built with the cli feature")
}

#[test]
fn cli_help_lists_subcommands() {
    mt5_bridge()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("stats"))
        .stdout(predicate::str::contains("tail"))
        .stdout(predicate::str::contains("wal"));
}

#[test]
fn doctor_help_lists_options() {
    mt5_bridge()
        .args(["doctor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dll"))
        .stdout(predicate::str::contains("--connect"))
        .stdout(predicate::str::contains("--wal"));
}

#[test]
fn stats_help_has_no_required_args() {
    mt5_bridge()
        .args(["stats", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Print bridge statistics"));
}

#[test]
fn tail_help_lists_options() {
    mt5_bridge()
        .args(["tail", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--symbol"))
        .stdout(predicate::str::contains("--count"));
}

#[test]
fn wal_verify_help_requires_path() {
    mt5_bridge()
        .args(["wal", "verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Path to the WAL store"));
}

#[test]
fn no_subcommand_shows_help_and_errors() {
    mt5_bridge()
        .assert()
        .failure()
        .stderr(predicate::str::contains("doctor"))
        .stderr(predicate::str::contains("Usage"));
}

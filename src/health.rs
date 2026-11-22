//! Health and readiness reporting for operational tooling.
//!
//! This module is intentionally free of CLI dependencies so it can be reused
//! by any binary or integration that wants to report bridge health.
//!
//! The `doctor` function aggregates checks across the DLL,
//! named-pipe auth slot, wire protocol, account, reconciliation state,
//! bridge statistics, and WAL store.

use crate::client::validate_dll_path;
use crate::error::Result;
use crate::ffi::PROTOCOL_VERSION;
use crate::reconciliation::{LifecycleState, OrderManager};
use crate::types::{AccountInfo, BridgeStats};
use std::path::{Path, PathBuf};

/// Outcome of a single health check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HealthStatus {
    /// The check passed.
    #[default]
    Ok,
    /// The check passed but with a noteworthy condition.
    Warn,
    /// The check failed.
    Fail,
}

impl HealthStatus {
    /// Worst-case ordering: `Fail > Warn > Ok`.
    fn rank(self) -> u8 {
        match self {
            HealthStatus::Ok => 0,
            HealthStatus::Warn => 1,
            HealthStatus::Fail => 2,
        }
    }
}

/// A single checked item with a human-readable detail.
#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub name: &'static str,
    pub status: HealthStatus,
    pub detail: String,
}

impl HealthCheck {
    pub fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: HealthStatus::Ok,
            detail: detail.into(),
        }
    }

    pub fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: HealthStatus::Warn,
            detail: detail.into(),
        }
    }

    pub fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: HealthStatus::Fail,
            detail: detail.into(),
        }
    }
}

/// Aggregated health report produced by [`doctor`].
#[derive(Debug, Clone)]
pub struct HealthReport {
    pub checks: Vec<HealthCheck>,
    pub overall: HealthStatus,
}

impl HealthReport {
    /// True when every check is `Ok`.
    pub fn is_healthy(&self) -> bool {
        self.checks.iter().all(|c| c.status == HealthStatus::Ok)
    }

    /// Merge `other` into `self`, keeping the worst overall status.
    pub fn merge(mut self, other: HealthReport) -> Self {
        self.checks.extend(other.checks);
        self.overall = if self.overall.rank() >= other.overall.rank() {
            self.overall
        } else {
            other.overall
        };
        self
    }
}

/// Result of checking the bridge DLL on disk.
pub fn check_dll(path: &Path) -> HealthCheck {
    match validate_dll_path(path) {
        Ok(_) => HealthCheck::ok("dll", path.to_string_lossy().to_string()),
        Err(err) => HealthCheck::fail("dll", err.to_string()),
    }
}

/// Result of checking the named-pipe auth / connection slot.
pub fn check_pipe(connected: bool) -> HealthCheck {
    if connected {
        HealthCheck::ok("pipe", "connection slot held")
    } else {
        HealthCheck::warn("pipe", "no live client connection")
    }
}

/// Result of checking the negotiated wire protocol version.
pub fn check_protocol(protocol: u32) -> HealthCheck {
    if protocol == PROTOCOL_VERSION {
        HealthCheck::ok("protocol", format!("v{protocol}"))
    } else {
        HealthCheck::fail(
            "protocol",
            format!("expected v{}, got v{}", PROTOCOL_VERSION, protocol),
        )
    }
}

/// Result of checking account info.
pub fn check_account(result: &Result<AccountInfo>) -> HealthCheck {
    match result {
        Ok(account) => HealthCheck::ok(
            "account",
            format!(
                "balance={:.2} equity={:.2} margin_level={:.1}",
                account.balance,
                account.equity,
                account.margin_level().unwrap_or(0.0)
            ),
        ),
        Err(err) => HealthCheck::fail("account", err.to_string()),
    }
}

/// Result of checking reconciliation state.
pub fn check_reconciliation(manager: &OrderManager) -> HealthCheck {
    let lifecycle = manager.lifecycle();
    let unresolved = manager.unresolved_orders().len();
    let status = match lifecycle {
        LifecycleState::Ready if unresolved == 0 => HealthStatus::Ok,
        LifecycleState::Stopped => HealthStatus::Fail,
        _ => HealthStatus::Warn,
    };
    HealthCheck {
        name: "reconciliation",
        status,
        detail: format!("lifecycle={lifecycle:?} unresolved={unresolved}"),
    }
}

/// Result of checking bridge statistics.
pub fn check_stats(stats: Option<&BridgeStats>) -> HealthCheck {
    match stats {
        Some(s) => {
            let drops = s.tick_drops + s.trade_drops + s.book_drops + s.bar_drops;
            let depth = s.tick_queue_depth + s.trade_queue_depth;
            HealthCheck::ok(
                "stats",
                format!(
                    "uptime={}s requests={} drops={} depth={} last_error={}",
                    s.uptime_secs, s.requests_total, drops, depth, s.last_error
                ),
            )
        }
        None => HealthCheck::warn("stats", "bridge stats export unavailable"),
    }
}

/// Result of checking the WAL store.
#[cfg(feature = "wal")]
pub fn check_wal(path: &Path) -> HealthCheck {
    match crate::wal::WalOrderStore::open(path) {
        Ok(_) => HealthCheck::ok("wal", path.to_string_lossy().to_string()),
        Err(err) => HealthCheck::fail("wal", err.to_string()),
    }
}

/// Run the full `doctor` suite.
///
/// `client` is an optional already-connected [`crate::Mt5Client`]; when `None` the
/// connection-dependent checks (protocol, account, stats) report as unavailable
/// instead of failing. `manager` is the reconciliation manager to inspect.
/// `wal_path`, when `Some`, is opened to verify the WAL store (requires the
/// `wal` feature).
///
/// This function performs no connection itself; the caller (typically the CLI
/// binary) connects the client before invoking `doctor`.
pub fn doctor(
    dll_path: &Path,
    client: Option<&crate::client::Mt5Client>,
    manager: &OrderManager,
    wal_path: Option<&Path>,
) -> HealthReport {
    let mut checks = Vec::new();
    checks.push(check_dll(dll_path));

    match client {
        Some(client) => {
            checks.push(check_pipe(true));
            match client.ping() {
                Ok(ping) => checks.push(check_protocol(ping.protocol)),
                Err(err) => checks.push(HealthCheck::fail("protocol", err.to_string())),
            }
            match client.account_info() {
                Ok(account) => checks.push(check_account(&Ok(account))),
                Err(err) => checks.push(check_account(&Err(err))),
            }
            match client.stats() {
                Ok(stats) => checks.push(check_stats(Some(&stats))),
                Err(_) => checks.push(check_stats(None::<&BridgeStats>)),
            }
        }
        None => {
            checks.push(check_pipe(false));
            checks.push(HealthCheck::warn("protocol", "client not connected"));
            checks.push(HealthCheck::warn("account", "client not connected"));
            checks.push(check_stats(None::<&BridgeStats>));
        }
    }

    checks.push(check_reconciliation(manager));

    #[cfg(feature = "wal")]
    if let Some(path) = wal_path {
        checks.push(check_wal(path));
    }
    #[cfg(not(feature = "wal"))]
    if wal_path.is_some() {
        checks.push(HealthCheck::warn(
            "wal",
            "wal feature disabled; store not opened",
        ));
    }

    let overall = checks
        .iter()
        .map(|c| c.status)
        .fold(HealthStatus::Ok, |acc, status| {
            if acc.rank() >= status.rank() {
                acc
            } else {
                status
            }
        });
    HealthReport { checks, overall }
}

/// Resolve the DLL path from an explicit override, the `MT5_DLL_PATH`
/// environment variable, or a `mt5_bridge.dll` adjacent to the executable.
pub fn resolve_dll_path(override_path: Option<&Path>) -> Option<PathBuf> {
    override_path
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("MT5_DLL_PATH").map(PathBuf::from))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("mt5_bridge.dll")))
        })
        .filter(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Mt5Error;
    use crate::reconciliation::StrategyMagic;
    use crate::types::AccountInfo;

    fn ok_account() -> AccountInfo {
        AccountInfo {
            balance: 10_000.0,
            equity: 10_250.50,
            margin: 1_000.0,
            free_margin: 9_000.0,
            leverage: 100.0,
            currency: "USD".into(),
        }
    }

    fn test_manager() -> OrderManager {
        OrderManager::new(StrategyMagic::const_new(20240101).unwrap())
    }

    #[test]
    fn health_report_is_healthy_when_all_checks_ok() {
        let report = HealthReport {
            checks: vec![
                HealthCheck::ok("dll", "mt5_bridge.dll"),
                HealthCheck::ok("pipe", "connection slot held"),
            ],
            overall: HealthStatus::Ok,
        };
        assert!(report.is_healthy());
    }

    #[test]
    fn health_report_is_not_healthy_when_any_check_fails() {
        let report = HealthReport {
            checks: vec![
                HealthCheck::ok("dll", "mt5_bridge.dll"),
                HealthCheck::fail("pipe", "connection refused"),
            ],
            overall: HealthStatus::Fail,
        };
        assert!(!report.is_healthy());
    }

    #[test]
    fn merge_keeps_worst_status() {
        let ok = HealthReport {
            checks: vec![HealthCheck::ok("a", "ok")],
            overall: HealthStatus::Ok,
        };
        let fail = HealthReport {
            checks: vec![HealthCheck::fail("b", "fail")],
            overall: HealthStatus::Fail,
        };
        assert!(!ok.clone().merge(fail).is_healthy());
    }

    #[test]
    fn check_protocol_matches_current_version() {
        let check = check_protocol(PROTOCOL_VERSION);
        assert_eq!(check.status, HealthStatus::Ok);
        assert!(check.detail.contains(&PROTOCOL_VERSION.to_string()));
    }

    #[test]
    fn check_protocol_mismatch_fails() {
        let check = check_protocol(PROTOCOL_VERSION + 1);
        assert_eq!(check.status, HealthStatus::Fail);
    }

    #[test]
    fn check_account_ok_reports_balance_and_equity() {
        let account = ok_account();
        let check = check_account(&Ok(account));
        assert_eq!(check.status, HealthStatus::Ok);
        assert!(check.detail.contains("10000.00"));
        assert!(check.detail.contains("10250.50"));
    }

    #[test]
    fn check_account_err_fails() {
        let check = check_account(&Err(Mt5Error::PipeDisconnected));
        assert_eq!(check.status, HealthStatus::Fail);
    }

    #[test]
    fn check_reconciliation_warns_when_not_ready() {
        let manager = test_manager();
        let check = check_reconciliation(&manager);
        assert_eq!(check.status, HealthStatus::Warn);
    }

    #[test]
    fn check_reconciliation_ok_when_ready_and_settled() {
        let mut manager = test_manager();
        manager.set_lifecycle(LifecycleState::Ready);
        let check = check_reconciliation(&manager);
        assert_eq!(check.status, HealthStatus::Ok);
    }

    #[test]
    fn check_stats_none_warns() {
        let check = check_stats(None);
        assert_eq!(check.status, HealthStatus::Warn);
    }

    #[test]
    fn check_stats_some_ok() {
        let stats = BridgeStats {
            uptime_secs: 3600,
            requests_total: 42,
            tick_drops: 0,
            trade_drops: 0,
            book_drops: 0,
            bar_drops: 0,
            tick_queue_depth: 0,
            trade_queue_depth: 0,
            last_error: 0,
            protocol: PROTOCOL_VERSION,
        };
        let check = check_stats(Some(&stats));
        assert_eq!(check.status, HealthStatus::Ok);
    }

    #[test]
    fn doctor_without_client_reports_warns_not_fails() {
        let manager = test_manager();
        let report = doctor(std::path::Path::new(""), None, &manager, None);
        // No live client: protocol/account/stats warn, pipe warns, dll fails (empty path).
        assert!(!report.is_healthy());
        assert_eq!(report.overall, HealthStatus::Fail);
        assert!(report
            .checks
            .iter()
            .any(|c| c.name == "dll" && c.status == HealthStatus::Fail));
        assert!(report
            .checks
            .iter()
            .any(|c| c.name == "pipe" && c.status == HealthStatus::Warn));
    }

    #[test]
    fn doctor_degraded_mode_reports_warns_for_missing_client() {
        let manager = test_manager();
        let report = doctor(
            std::path::Path::new("/nonexistent/bridge.dll"),
            None,
            &manager,
            None,
        );
        assert!(!report.is_healthy());
        // DLL is missing (fail), but connection-dependent checks warn rather than fail.
        assert!(report
            .checks
            .iter()
            .any(|c| c.name == "protocol" && c.status == HealthStatus::Warn));
        assert!(report
            .checks
            .iter()
            .any(|c| c.name == "account" && c.status == HealthStatus::Warn));
        assert!(report
            .checks
            .iter()
            .any(|c| c.name == "stats" && c.status == HealthStatus::Warn));
        assert!(report
            .checks
            .iter()
            .any(|c| c.name == "pipe" && c.status == HealthStatus::Warn));
        assert!(report
            .checks
            .iter()
            .any(|c| c.name == "reconciliation" && c.status == HealthStatus::Warn));
    }

    #[test]
    fn resolve_dll_path_prefers_override() {
        let existing = std::env::temp_dir().join("health_test_override_dll");
        std::fs::write(&existing, b"fake").ok();
        let override_path = Path::new(&existing);
        assert_eq!(
            resolve_dll_path(Some(override_path)),
            Some(PathBuf::from(&existing))
        );
        let _ = std::fs::remove_file(&existing);
    }
}

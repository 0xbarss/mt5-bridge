use crate::error::{Mt5Error, Result};
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io::copy;
use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use super::client::Mt5Client;

impl Mt5Client {
    /// Locate `mt5_bridge.dll` next to the running executable, if present.
    ///
    /// This is the only implicit DLL location allowed. The client never falls back
    /// to a bare DLL name resolved through the OS ambient search order.
    pub(crate) fn exe_adjacent_dll_path() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.join("mt5_bridge.dll")))
            .filter(|dll_path| dll_path.is_file())
    }

    /// Validate that `path` is an absolute path to a regular file that is not
    /// stored in a world-writable directory, then canonicalize it so the loader
    /// never resolves it through the OS ambient DLL search order.
    pub(crate) fn validate_dll_path(path: &Path) -> Result<PathBuf> {
        if path.is_relative() {
            return Err(Mt5Error::InvalidDllPath {
                path: path.display().to_string(),
                reason: "relative DLL paths are refused; set MT5_DLL_PATH to an absolute path"
                    .to_string(),
            });
        }

        let resolved = path.to_path_buf();

        if !resolved.is_file() {
            return Err(Mt5Error::InvalidDllPath {
                path: resolved.display().to_string(),
                reason: "DLL file not found".to_string(),
            });
        }

        if let Some(parent) = resolved.parent() {
            if crate::client::pipe::is_world_writable_dir(parent) {
                return Err(Mt5Error::InvalidDllPath {
                    path: resolved.display().to_string(),
                    reason: format!(
                        "DLL directory {parent:?} is world-writable; refusing to load the DLL"
                    ),
                });
            }
        }

        Ok(resolved)
    }

    /// Verify the SHA-256 hash of the DLL against an expected value.
    ///
    /// The expected hash is read from the `MT5_DLL_SHA256` environment variable.
    /// If the variable is not set, verification is skipped with a prominent
    /// warning (the operator has not pinned the bridge binary).
    /// The hash should be provided as a lowercase hexadecimal string (64 characters).
    pub(crate) fn verify_dll_hash(path: &Path) -> Result<()> {
        let require_hash = std::env::var("MT5_DLL_REQUIRE_HASH")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                )
            })
            .unwrap_or(false);
        let Ok(expected_hex) = std::env::var("MT5_DLL_SHA256") else {
            if require_hash {
                return Err(Mt5Error::InvalidDllPath {
                    path: path.display().to_string(),
                    reason: "MT5_DLL_REQUIRE_HASH is enabled but MT5_DLL_SHA256 is not set"
                        .to_string(),
                });
            }
            warn!(
                dll = %path.display(),
                "MT5_DLL_SHA256 is not set; the bridge DLL is loaded without an integrity check"
            );
            return Ok(());
        };
        let expected_hex = expected_hex.trim();
        if expected_hex.is_empty() {
            if require_hash {
                return Err(Mt5Error::InvalidDllPath {
                    path: path.display().to_string(),
                    reason: "MT5_DLL_REQUIRE_HASH is enabled but MT5_DLL_SHA256 is empty"
                        .to_string(),
                });
            }
            warn!(
                dll = %path.display(),
                "MT5_DLL_SHA256 is empty; the bridge DLL is loaded without an integrity check"
            );
            return Ok(());
        }
        if expected_hex.len() != 64 {
            return Err(Mt5Error::InvalidDllPath {
                path: path.display().to_string(),
                reason: format!(
                    "MT5_DLL_SHA256 must be a 64-character hex string, got {} characters",
                    expected_hex.len()
                ),
            });
        }
        let expected_bytes = hex::decode(expected_hex).map_err(|_| Mt5Error::InvalidDllPath {
            path: path.display().to_string(),
            reason: "MT5_DLL_SHA256 is not valid hex".to_string(),
        })?;

        let mut hasher = Sha256::new();
        let mut file = File::open(path).map_err(|source| Mt5Error::InvalidDllPath {
            path: path.display().to_string(),
            reason: format!("could not open DLL for hashing: {source}"),
        })?;
        copy(&mut file, &mut hasher).map_err(|source| Mt5Error::InvalidDllPath {
            path: path.display().to_string(),
            reason: format!("could not read DLL for hashing: {source}"),
        })?;
        let actual = hasher.finalize();

        if actual.as_slice() != expected_bytes {
            return Err(Mt5Error::InvalidDllPath {
                path: path.display().to_string(),
                reason: format!(
                    "DLL SHA-256 mismatch: expected {}, got {}",
                    expected_hex,
                    hex::encode(actual)
                ),
            });
        }
        info!("DLL SHA-256 verified: {}", path.display());
        Ok(())
    }

    /// Require operator-selected exports before using the bridge. This keeps
    /// compatibility with older DLLs by default while allowing deployments to
    /// make safety-sensitive optional APIs mandatory.
    pub(crate) fn verify_required_exports(lib: &libloading::Library) -> Result<()> {
        let Some(raw) = std::env::var_os("MT5_DLL_REQUIRED_EXPORTS") else {
            return Ok(());
        };
        for export in raw.to_string_lossy().split(',').map(str::trim) {
            if export.is_empty() {
                continue;
            }
            let symbol = CString::new(export).map_err(|_| Mt5Error::InvalidDllPath {
                path: String::new(),
                reason: format!(
                    "MT5_DLL_REQUIRED_EXPORTS contains an invalid export name: {export:?}"
                ),
            })?;
            let symbol = symbol.as_bytes_with_nul();
            let present = unsafe { lib.get::<*const c_void>(symbol) }.is_ok();
            if !present {
                return Err(Mt5Error::InvalidDllPath {
                    path: String::new(),
                    reason: format!("required MT5 DLL export '{export}' was not found"),
                });
            }
        }
        Ok(())
    }
}

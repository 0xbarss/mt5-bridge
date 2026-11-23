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
    pub fn validate_dll_path(path: &Path) -> Result<PathBuf> {
        crate::client::dll_path::validate_dll_path(path)
    }

    /// Verify the SHA-256 hash of the DLL against an expected value.
    ///
    /// The expected hash is read from the `MT5_DLL_SHA256` environment variable,
    /// falling back to a `<dll_name>.sha256` manifest file next to the DLL.
    ///
    /// `MT5_DLL_REQUIRE_HASH` controls whether a hash is mandatory. It defaults
    /// to `true` in release builds (`cfg!(not(debug_assertions))`) unless
    /// explicitly set to `false`, and to `false` in debug builds.
    ///
    /// Set `MT5_DLL_ALLOW_UNPINNED=1` to skip hash verification entirely
    /// (development escape hatch).
    pub(crate) fn verify_dll_hash(path: &Path) -> Result<()> {
        // Development escape hatch: skip hash verification entirely.
        let allow_unpinned = std::env::var("MT5_DLL_ALLOW_UNPINNED")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                )
            })
            .unwrap_or(false);
        if allow_unpinned {
            warn!(
                dll = %path.display(),
                "MT5_DLL_ALLOW_UNPINNED is set; DLL hash verification is skipped"
            );
            return Ok(());
        }

        // MT5_DLL_REQUIRE_HASH defaults to true in release builds unless
        // explicitly set to false.
        let require_hash = match std::env::var("MT5_DLL_REQUIRE_HASH") {
            Ok(value) => matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            ),
            Err(_) => cfg!(not(debug_assertions)),
        };

        // Read the expected hash from MT5_DLL_SHA256, falling back to a
        // <dll_name>.sha256 manifest file next to the DLL.
        let expected_hex = std::env::var("MT5_DLL_SHA256")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                path.parent()
                    .and_then(|parent| {
                        path.file_name()
                            .map(|name| parent.join(format!("{}.sha256", name.to_string_lossy())))
                    })
                    .and_then(|manifest_path| std::fs::read_to_string(&manifest_path).ok())
            });

        let expected_hex = match expected_hex {
            Some(hex) => hex.trim().to_string(),
            None => {
                if require_hash {
                    return Err(Mt5Error::InvalidDllPath {
                        path: path.display().to_string(),
                        reason: "MT5_DLL_REQUIRE_HASH is enabled but no DLL hash was provided \
                               (set MT5_DLL_SHA256 or place a <dll>.sha256 manifest next to the DLL)"
                            .to_string(),
                    });
                }
                warn!(
                    dll = %path.display(),
                    "no DLL hash provided; the bridge DLL is loaded without an integrity check"
                );
                return Ok(());
            }
        };

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
        let expected_bytes = hex::decode(&expected_hex).map_err(|_| Mt5Error::InvalidDllPath {
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

#[cfg(test)]
mod tests {
    use crate::client::client_impl::client::Mt5Client;
    use std::env;

    // Serializes hash tests: they mutate process-global env vars, so they must
    // not run concurrently with one another.
    static HASH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_hash_test_lock() -> std::sync::MutexGuard<'static, ()> {
        HASH_TEST_LOCK
            .lock()
            .expect("hash test lock must not be poisoned")
    }

    fn compute_sha256(content: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content);
        hex::encode(hasher.finalize())
    }

    /// Temporarily set (or remove) an env var, run `f`, then restore the previous value.
    fn with_env_var<F>(key: &str, value: Option<&str>, f: F)
    where
        F: FnOnce(),
    {
        let prev: Option<String> = std::env::var(key).ok();
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        f();
        match prev {
            Some(v) => std::env::set_var(key, &v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn verify_dll_hash_accepts_correct_hash() {
        let _lock = with_hash_test_lock();
        let dir = env::temp_dir().join(format!("mt5_hash_test_valid_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let dll = dir.join("mt5_bridge.dll");
        std::fs::write(&dll, b"dummy dll content").expect("dummy dll must be writable");

        let hash = compute_sha256(b"dummy dll content");
        with_env_var("MT5_DLL_SHA256", Some(&hash), || {
            with_env_var("MT5_DLL_REQUIRE_HASH", Some("true"), || {
                assert!(
                    Mt5Client::verify_dll_hash(&dll).is_ok(),
                    "correct hash should be accepted"
                );
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_dll_hash_rejects_wrong_hash() {
        let _lock = with_hash_test_lock();
        let dir = env::temp_dir().join(format!("mt5_hash_test_wrong_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let dll = dir.join("mt5_bridge.dll");
        std::fs::write(&dll, b"dummy dll content").expect("dummy dll must be writable");

        with_env_var(
            "MT5_DLL_SHA256",
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            || {
                with_env_var("MT5_DLL_REQUIRE_HASH", Some("true"), || {
                    assert!(
                        Mt5Client::verify_dll_hash(&dll).is_err(),
                        "wrong hash should be rejected"
                    );
                });
            },
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_dll_hash_requires_hash_when_flag_enabled() {
        let _lock = with_hash_test_lock();
        let dir = env::temp_dir().join(format!("mt5_hash_test_require_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let dll = dir.join("mt5_bridge.dll");
        std::fs::write(&dll, b"dummy dll content").expect("dummy dll must be writable");

        // Ensure no .sha256 manifest exists next to the DLL.
        let manifest = dir.join("mt5_bridge.dll.sha256");
        let _ = std::fs::remove_file(&manifest);

        // Ensure the escape hatch is not set.
        std::env::remove_var("MT5_DLL_ALLOW_UNPINNED");

        with_env_var("MT5_DLL_SHA256", None, || {
            with_env_var("MT5_DLL_REQUIRE_HASH", Some("true"), || {
                assert!(
                    Mt5Client::verify_dll_hash(&dll).is_err(),
                    "missing hash should be rejected when required"
                );
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_dll_hash_allows_missing_hash_when_not_required() {
        let _lock = with_hash_test_lock();
        let dir = env::temp_dir().join(format!("mt5_hash_test_optional_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let dll = dir.join("mt5_bridge.dll");
        std::fs::write(&dll, b"dummy dll content").expect("dummy dll must be writable");

        with_env_var("MT5_DLL_SHA256", None, || {
            with_env_var("MT5_DLL_REQUIRE_HASH", Some("false"), || {
                assert!(
                    Mt5Client::verify_dll_hash(&dll).is_ok(),
                    "missing hash should be allowed when not required"
                );
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_dll_hash_allow_unpinned_bypasses_requirement() {
        let _lock = with_hash_test_lock();
        let dir = env::temp_dir().join(format!("mt5_hash_test_escape_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let dll = dir.join("mt5_bridge.dll");
        std::fs::write(&dll, b"dummy dll content").expect("dummy dll must be writable");

        with_env_var("MT5_DLL_SHA256", None, || {
            with_env_var("MT5_DLL_REQUIRE_HASH", Some("true"), || {
                with_env_var("MT5_DLL_ALLOW_UNPINNED", Some("1"), || {
                    assert!(
                        Mt5Client::verify_dll_hash(&dll).is_ok(),
                        "allow-unpinned should bypass hash requirement"
                    );
                });
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_dll_hash_reads_manifest_next_to_dll() {
        let _lock = with_hash_test_lock();
        let dir = env::temp_dir().join(format!("mt5_hash_test_manifest_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let dll = dir.join("mt5_bridge.dll");
        std::fs::write(&dll, b"dummy dll content").expect("dummy dll must be writable");

        let hash = compute_sha256(b"dummy dll content");
        let manifest = dir.join("mt5_bridge.dll.sha256");
        std::fs::write(&manifest, &hash).expect("manifest must be writable");

        with_env_var("MT5_DLL_SHA256", None, || {
            with_env_var("MT5_DLL_REQUIRE_HASH", Some("true"), || {
                assert!(
                    Mt5Client::verify_dll_hash(&dll).is_ok(),
                    "hash from manifest next to DLL should be accepted"
                );
            });
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Tests for `mt5_bridge::client`.

#![allow(clippy::disallowed_methods)]

use crate::client::client_impl::client::{Mt5Client, CLIENT_ACTIVE};
use crate::client::pipe::validate_pipe_name;
use crate::error::Mt5Error;
use std::sync::atomic::Ordering;

// Serializes the tests below: they exercise the process-wide `CLIENT_ACTIVE`
// slot, so they must not run concurrently with one another (the default test
// harness runs threads in parallel, which would otherwise make a rejected
// connect observe another test's in-flight connection as `AlreadyConnected`).
static CLIENT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_client_lock() -> std::sync::MutexGuard<'static, ()> {
    CLIENT_TEST_LOCK
        .lock()
        .expect("test lock must not be poisoned")
}

#[test]
fn connect_with_dll_rejects_relative_paths() {
    let _lock = with_client_lock();
    for relative in ["mt5_bridge.dll", "./mt5_bridge.dll", "../mt5_bridge.dll"] {
        let err = match Mt5Client::connect_with_dll(relative, 0, "", "") {
            Ok(_) => panic!("{relative:?} must not be accepted as a DLL path"),
            Err(err) => err,
        };
        assert!(
            matches!(err, Mt5Error::InvalidDllPath { .. }),
            "{relative:?} must be rejected as a relative DLL path, got {err:?}"
        );
        assert!(
            !CLIENT_ACTIVE.load(Ordering::SeqCst),
            "CLIENT_ACTIVE must be reset after a rejected {relative:?}"
        );
    }
}

#[test]
fn connect_with_dll_rejects_nonexistent_absolute_path() {
    let _lock = with_client_lock();
    let abs = std::env::temp_dir().join("no_such_mt5_bridge_dll_xyz.dll");
    let err = match Mt5Client::connect_with_dll(&abs, 0, "", "") {
        Ok(_) => panic!("nonexistent absolute path must not connect"),
        Err(err) => err,
    };
    assert!(
        matches!(err, Mt5Error::InvalidDllPath { .. }),
        "nonexistent absolute path must be rejected, got {err:?}"
    );
    assert!(
        !CLIENT_ACTIVE.load(Ordering::SeqCst),
        "CLIENT_ACTIVE must be reset after a rejected absolute path"
    );
}

#[cfg(unix)]
#[test]
fn connect_with_dll_rejects_world_writable_directory() {
    let _lock = with_client_lock();
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("mt5_bridge_ww_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test temp dir must be creatable");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
        .expect("temp dir must be set world-writable");
    let dll = dir.join("mt5_bridge.dll");
    std::fs::write(&dll, b"not a real dll").expect("dummy dll must be writable");

    let err = match Mt5Client::connect_with_dll(&dll, 0, "", "") {
        Ok(_) => panic!("world-writable DLL directory must not be accepted"),
        Err(err) => err,
    };
    assert!(
        matches!(err, Mt5Error::InvalidDllPath { .. }),
        "world-writable DLL directory must be refused, got {err:?}"
    );
    assert!(
        !CLIENT_ACTIVE.load(Ordering::SeqCst),
        "CLIENT_ACTIVE must be reset after a refused world-writable path"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dll_load_failure_releases_client_active() {
    let _lock = with_client_lock();
    // A file that exists but is not a valid DLL, so loading must fail.
    // The directory must not be world-writable, or the world-writable
    // directory check fires before the loader ever runs.
    let dir = std::env::temp_dir().join(format!("mt5_bridge_dummy_dir_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test temp dir must be creatable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("test temp dir permissions must be settable");
    }
    let dummy = dir.join(format!("mt5_bridge_dummy_{}.dll", std::process::id()));
    std::fs::write(
        &dummy,
        b"this is definitely not a valid PE/ELF/DYLD library",
    )
    .expect("dummy dll must be writable");

    for _ in 0..2 {
        let err = match Mt5Client::connect_with_dll(&dummy, 0, "", "") {
            Ok(_) => panic!("non-DLL file must not connect"),
            Err(err) => err,
        };
        assert!(
            matches!(err, Mt5Error::DllLoadError { .. }),
            "expected a DLL load error, got {err:?}"
        );
        assert!(
            !CLIENT_ACTIVE.load(Ordering::SeqCst),
            "CLIENT_ACTIVE must be reset after a partial load"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pipe_name_validation_accepts_only_plain_names() {
    for valid in ["mt5bridge", "mt5bridge_fx", "a", "A-1.2_3", &"x".repeat(64)] {
        assert!(
            validate_pipe_name(valid).is_ok(),
            "{valid:?} should be accepted"
        );
    }

    for invalid in [
        "",
        "mt5bridge\\evil",
        "..\\..\\pipe\\x",
        "pipe/name",
        "name with space",
        "nul\0byte",
        &"x".repeat(65),
    ] {
        assert!(
            validate_pipe_name(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
    }
}

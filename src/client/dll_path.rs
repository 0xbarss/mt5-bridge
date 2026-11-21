use crate::error::{Mt5Error, Result};
use std::path::{Path, PathBuf};

/// Validates that `path` is an absolute path to an existing regular file that is not
/// stored in a world-writable directory, then canonicalizes it to avoid ambient DLL
/// search order hijacking.
///
/// On Windows, paths are canonicalized using `dunce::canonicalize` to avoid UNC (`\\?\`)
/// path prefixes that may be unsupported by legacy loader APIs.
pub fn validate_dll_path(path: &Path) -> Result<PathBuf> {
    if path.is_relative() {
        return Err(Mt5Error::InvalidDllPath {
            path: path.display().to_string(),
            reason: "relative DLL paths are refused; set MT5_DLL_PATH to an absolute path"
                .to_string(),
        });
    }

    let canonical = dunce::canonicalize(path).map_err(|e| Mt5Error::InvalidDllPath {
        path: path.display().to_string(),
        reason: format!("cannot resolve DLL path: {e}"),
    })?;

    if !canonical.is_file() {
        return Err(Mt5Error::InvalidDllPath {
            path: canonical.display().to_string(),
            reason: "DLL path does not point to a regular file".to_string(),
        });
    }

    if let Some(parent) = path.parent() {
        if is_world_writable_dir(parent) {
            return Err(Mt5Error::InvalidDllPath {
                path: path.display().to_string(),
                reason: format!(
                    "DLL directory {parent:?} is world-writable; refusing to load the DLL"
                ),
            });
        }
    }

    if let Some(parent) = canonical.parent() {
        if is_world_writable_dir(parent) {
            return Err(Mt5Error::InvalidDllPath {
                path: canonical.display().to_string(),
                reason: format!(
                    "DLL directory {parent:?} is world-writable; refusing to load the DLL"
                ),
            });
        }
    }

    Ok(canonical)
}

#[cfg(unix)]
pub(crate) fn is_world_writable_dir(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match dir.metadata() {
        Ok(md) => {
            let mode = md.permissions().mode() & 0o777;
            mode & 0o002 != 0
        }
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub(crate) fn is_world_writable_dir(_dir: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_path_rejected() {
        for rel in [
            "mt5_bridge.dll",
            "./mt5_bridge.dll",
            "../lib/mt5_bridge.dll",
        ] {
            let res = validate_dll_path(Path::new(rel));
            assert!(
                matches!(res, Err(Mt5Error::InvalidDllPath { .. })),
                "expected InvalidDllPath for {rel:?}, got {res:?}"
            );
        }
    }

    #[test]
    fn test_non_file_rejected() {
        let temp_dir = std::env::temp_dir();
        let res = validate_dll_path(&temp_dir);
        assert!(
            matches!(res, Err(Mt5Error::InvalidDllPath { .. })),
            "expected InvalidDllPath for directory, got {res:?}"
        );
    }

    #[test]
    fn test_nonexistent_file_rejected() {
        let bogus = std::env::temp_dir().join("definitely_nonexistent_dll_123456.dll");
        let res = validate_dll_path(&bogus);
        assert!(
            matches!(res, Err(Mt5Error::InvalidDllPath { .. })),
            "expected InvalidDllPath for nonexistent path, got {res:?}"
        );
    }

    #[test]
    fn test_valid_file_accepted() {
        let dir = std::env::temp_dir().join(format!("mt5_test_valid_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
                .expect("temp dir permissions must be settable");
        }
        let file = dir.join("dummy.dll");
        std::fs::write(&file, b"content").expect("dummy file must be writable");

        let canonical = validate_dll_path(&file).expect("valid file should pass validation");
        assert!(canonical.is_file());
        assert_eq!(
            canonical,
            dunce::canonicalize(&file).expect("canonicalize should succeed")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_world_writable_parent_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mt5_test_ww_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
            .expect("temp dir must be set world-writable");
        let file = dir.join("dummy.dll");
        std::fs::write(&file, b"content").expect("dummy file must be writable");

        let res = validate_dll_path(&file);
        assert!(
            matches!(res, Err(Mt5Error::InvalidDllPath { .. })),
            "expected InvalidDllPath for world-writable parent, got {res:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_resolves_to_canonical() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mt5_test_symlink_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("temp dir permissions must be settable");

        let target_file = dir.join("real.dll");
        std::fs::write(&target_file, b"content").expect("real file must be writable");

        let symlink_file = dir.join("link.dll");
        std::os::unix::fs::symlink(&target_file, &symlink_file).expect("symlink must be creatable");

        let canonical =
            validate_dll_path(&symlink_file).expect("symlink to valid file should pass validation");
        assert_eq!(
            canonical,
            dunce::canonicalize(&target_file).expect("canonicalize should succeed")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

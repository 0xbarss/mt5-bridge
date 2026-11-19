use crate::error::Result;
use std::path::{Path, PathBuf};

pub fn validate_dll_path(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|e| crate::error::Mt5Error::Other(format!("cannot resolve DLL path: {e}")))?;
    if !canonical.is_file() {
        return Err(crate::error::Mt5Error::Other(format!(
            "DLL path does not point to a file: {}",
            path.display()
        )));
    }
    Ok(canonical)
}

pub fn validate_pipe_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(crate::error::Mt5Error::Other(
            "pipe name must not be empty".into(),
        ));
    }
    if name.len() > 64 {
        return Err(crate::error::Mt5Error::Other(format!(
            "pipe name must be at most 64 characters (got {})",
            name.len()
        )));
    }
    if name.contains(std::path::MAIN_SEPARATOR) || name.contains('\\') {
        return Err(crate::error::Mt5Error::Other(format!(
            "pipe name must not contain path separators: {name}"
        )));
    }
    if name.starts_with('.') {
        return Err(crate::error::Mt5Error::Other(format!(
            "pipe name must not start with a dot: {name}"
        )));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return Err(crate::error::Mt5Error::Other(format!(
            "MT5_PIPE_NAME must match ^[A-Za-z0-9_.-]{{1,64}}$ (got {name:?})"
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn is_world_writable_dir(dir: &Path) -> bool {
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
pub(super) fn is_world_writable_dir(_dir: &Path) -> bool {
    false
}

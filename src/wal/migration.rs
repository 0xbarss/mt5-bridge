//! Legacy JSON-to-WAL migration and WAL-to-JSON conversion.

#[cfg(feature = "json-journal")]
use crate::error::Result;
#[cfg(feature = "json-journal")]
use crate::journal::OrderStore;
#[cfg(feature = "json-journal")]
use crate::wal::format::io_err;
#[cfg(feature = "json-journal")]
use crate::wal::store::WalOrderStore;
#[cfg(feature = "json-journal")]
use std::collections::HashMap;
#[cfg(feature = "json-journal")]
use std::io::Write;
#[cfg(feature = "json-journal")]
use std::path::Path;

/// Migrate a legacy `orders.json` file into an append-only WAL store.
///
/// Reads the legacy JSON envelope and checksum, writes an initial snapshot file
/// (`orders.snap`), creates `orders.wal`, and renames `orders.json` to `orders.json.bak`.
#[cfg(feature = "json-journal")]
pub fn migrate_json_to_wal(json_path: &Path, wal_path: &Path) -> Result<usize> {
    if !json_path.exists() {
        return Ok(0);
    }

    let json_store = crate::journal::JsonFileStore::new(json_path);
    let orders = json_store.load()?;
    let count = orders.len();

    let snap_path = wal_path.with_extension("snap");
    WalOrderStore::write_snapshot_file(&snap_path, &orders, &HashMap::new())?;

    // Create fresh WAL file
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(wal_path)
        .map_err(|e| io_err("create wal", wal_path, e))?;

    let mut header = [0u8; crate::wal::format::WAL_HEADER_SIZE];
    header[..8].copy_from_slice(crate::wal::format::WAL_MAGIC);
    header[8..12].copy_from_slice(&crate::wal::format::WAL_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&1u32.to_le_bytes());
    file.write_all(&header)
        .map_err(|e| io_err("write wal header", wal_path, e))?;
    file.sync_all()
        .map_err(|e| io_err("fsync wal", wal_path, e))?;

    // Rename legacy JSON to .bak
    let bak_path = json_path.with_extension("json.bak");
    std::fs::rename(json_path, &bak_path)
        .map_err(|e| io_err("backup legacy json", &bak_path, e))?;

    tracing::info!(
        orders_migrated = count,
        backup = %bak_path.display(),
        "Legacy orders.json successfully migrated to WAL snapshot"
    );
    Ok(count)
}

/// Convert a WAL store back to a standalone legacy JSON file.
#[cfg(feature = "json-journal")]
pub fn wal_to_json(wal_path: &Path, json_path: &Path) -> Result<usize> {
    let snap_path = wal_path.with_extension("snap");
    let (orders, _, _) = WalOrderStore::load_from_disk(wal_path, &snap_path)?;
    let count = orders.len();

    let json_store = crate::journal::JsonFileStore::new(json_path);
    json_store.save(&orders)?;
    Ok(count)
}

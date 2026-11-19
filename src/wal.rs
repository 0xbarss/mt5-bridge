//! Durable write-ahead log (WAL) order store.
//!
//! Provides high-throughput, low-latency append-only order journaling.
//! Pre-send intents (`OrderState::Submitting`) are written as small binary-framed
//! entries with synchronous `fsync`, while subsequent state transitions (e.g. fills,
//! modifications, closures) are appended incrementally without rewriting the entire state.
//!
//! Periodic atomic snapshots (`orders.snap`) and pruning keep WAL files compact and bounded.

use crate::error::{Mt5Error, Result};
use crate::journal::{JsonFileStore, OrderStore};
use crate::position_book::{PositionRef, PositionState};
use crate::sync::LockExt;
use crate::types::TrackedOrder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const WAL_MAGIC: &[u8; 8] = b"MT5WAL01";
const WAL_VERSION: u32 = 3;
const WAL_HEADER_SIZE: usize = 16;
const FRAME_HEADER_SIZE: usize = 18;

const REC_INTENT: u8 = 0x01;
const REC_UPDATE: u8 = 0x02;
const REC_SNAPSHOT: u8 = 0x03;
const REC_PRUNE: u8 = 0x04;
const REC_POSITION: u8 = 0x05;

/// Return type of [`WalOrderStore::load_from_disk`]: orders plus the persisted
/// position-state transitions needed to rebuild the `Unknown` position set.
type LoadResult = (
    Vec<TrackedOrder>,
    HashMap<u64, (PositionRef, PositionState)>,
);

const FLAG_SYNC: u8 = 0x01;

/// IEEE 802.3 CRC32 checksum (polynomial 0xEDB88320 in reflected form).
///
/// `poly` selects the polynomial: version 2 WALs used `0xED88_8320` (kept for
/// backwards compat); version 3+ use the standard IEEE polynomial `0xEDB8_8320`.
pub fn crc32(data: &[u8], poly: u32) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (poly & mask);
        }
    }
    !crc
}

/// CRC polynomial for the correct IEEE 802.3 CRC32 (reflected form).
const CRC_POLY_IEEE: u32 = 0xEDB8_8320;
/// CRC polynomial used by version 2 WALs (kept for backwards compat).
const CRC_POLY_V2: u32 = 0xED88_8320;

/// Return the CRC polynomial for a given WAL file version.
fn crc_poly_for_version(version: u32) -> u32 {
    match version {
        2 => CRC_POLY_V2,
        _ => CRC_POLY_IEEE,
    }
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> Mt5Error {
    Mt5Error::PersistenceError(format!("{what} '{}': {e}", path.display()))
}

/// Little-endian `u32` out of the first four bytes of `b`.
fn le_u32(b: &[u8]) -> Option<u32> {
    b.get(..4)?.try_into().ok().map(u32::from_le_bytes)
}

/// Little-endian `u64` out of the first eight bytes of `b`.
fn le_u64(b: &[u8]) -> Option<u64> {
    b.get(..8)?.try_into().ok().map(u64::from_le_bytes)
}

/// Configuration options for [`WalOrderStore`].
#[derive(Debug, Clone)]
pub struct WalConfig {
    /// Maximum number of WAL entries before triggering an atomic snapshot and truncation.
    pub max_wal_entries: usize,
    /// Maximum byte size of the WAL file before triggering compaction.
    pub max_wal_bytes: u64,
    /// Whether to synchronously fsync write-ahead intents (`record_intent`).
    pub sync_intent: bool,
    /// Automatically migrate from legacy `orders.json` if the WAL does not exist yet.
    pub migrate_legacy_json: bool,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            max_wal_entries: 2000,
            max_wal_bytes: 5 * 1024 * 1024, // 5 MB
            sync_intent: true,
            migrate_legacy_json: true,
        }
    }
}

impl WalConfig {
    /// Set maximum WAL entries before snapshot.
    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_wal_entries = max;
        self
    }

    /// Set maximum WAL bytes before snapshot.
    pub fn with_max_bytes(mut self, bytes: u64) -> Self {
        self.max_wal_bytes = bytes;
        self
    }

    /// Set synchronous fsync on order intent.
    pub fn with_sync_intent(mut self, sync: bool) -> Self {
        self.sync_intent = sync;
        self
    }

    /// Set legacy JSON auto-migration.
    pub fn with_migrate_legacy_json(mut self, migrate: bool) -> Self {
        self.migrate_legacy_json = migrate;
        self
    }
}

#[derive(Serialize, Deserialize)]
struct SnapshotEnvelope {
    format: String,
    version: u32,
    generation: u64,
    saved_at: i64,
    orders: Vec<TrackedOrder>,
    /// Persisted position-state transitions. All states are written; on load
    /// we apply last-write-wins and drop terminal states (Closed).
    #[serde(default)]
    positions: HashMap<u64, (PositionRef, PositionState)>,
}

struct WalInner {
    file: File,
    path: PathBuf,
    snap_path: PathBuf,
    seq: u64,
    entries_count: usize,
    config: WalConfig,
    /// CRC polynomial used by this WAL file, determined by its version at open.
    crc_poly: u32,
}

/// Append-only write-ahead log (WAL) order store implementing [`OrderStore`].
#[derive(Clone)]
pub struct WalOrderStore {
    inner: Arc<Mutex<WalInner>>,
    path: PathBuf,
}

impl WalOrderStore {
    /// Return the offset just past the last complete, CRC-valid frame.
    ///
    /// A partial frame header/payload or a bad CRC on the *final* frame is
    /// treated as a torn tail (the process died mid-append) and reported via
    /// the returned offset. Corruption with valid bytes after it is a real
    /// error: silently discarding the middle of the journal could resurrect
    /// orders that were later filled or cancelled.
    fn validated_wal_end(path: &Path, file_len: u64) -> Result<u64> {
        let mut file = File::open(path).map_err(|e| io_err("open wal", path, e))?;
        let mut offset = WAL_HEADER_SIZE as u64;

        // Read the WAL version from the header to select the CRC polynomial.
        let mut header = [0u8; WAL_HEADER_SIZE];
        file.seek(SeekFrom::Start(0))
            .map_err(|e| io_err("seek header", path, e))?;
        file.read_exact(&mut header)
            .map_err(|e| io_err("read header", path, e))?;
        let version = le_u32(&header[8..]).ok_or_else(|| {
            Mt5Error::PersistenceError(format!(
                "'{}' has a truncated WAL version field",
                path.display()
            ))
        })?;
        let crc_poly = crc_poly_for_version(version);

        while offset < file_len {
            if offset + FRAME_HEADER_SIZE as u64 > file_len {
                tracing::warn!(
                    wal = %path.display(),
                    offset,
                    "WAL ends with an incomplete frame header"
                );
                break;
            }

            file.seek(SeekFrom::Start(offset))
                .map_err(|e| io_err("seek frame", path, e))?;
            let mut frame_hdr = [0u8; FRAME_HEADER_SIZE];
            file.read_exact(&mut frame_hdr)
                .map_err(|e| io_err("read frame header", path, e))?;

            let (Some(len), Some(expected_crc)) =
                (le_u32(&frame_hdr[10..]), le_u32(&frame_hdr[14..]))
            else {
                tracing::warn!(
                    wal = %path.display(),
                    offset,
                    "WAL frame header ends before its length and CRC"
                );
                break;
            };
            let len = len as u64;
            let payload_start = offset + FRAME_HEADER_SIZE as u64;
            let frame_end = payload_start + len;
            if frame_end > file_len {
                tracing::warn!(
                    wal = %path.display(),
                    offset,
                    "WAL ends with a truncated frame payload"
                );
                break;
            }

            let mut payload = vec![0u8; len as usize];
            file.read_exact(&mut payload)
                .map_err(|e| io_err("read payload", path, e))?;
            let actual_crc = crc32(&payload, crc_poly);
            if actual_crc != expected_crc {
                if frame_end == file_len {
                    tracing::warn!(
                        wal = %path.display(),
                        offset,
                        "WAL ends with a frame whose CRC does not match"
                    );
                    break;
                }
                return Err(Mt5Error::PersistenceError(format!(
                    "WAL '{}' has a corrupt frame at offset {offset} followed by more data",
                    path.display()
                )));
            }

            offset = frame_end;
        }

        Ok(offset)
    }

    /// Open or create a WAL store at `path` using default configuration.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Self::open_with_config(path, WalConfig::default())
    }

    /// Open or create a WAL store at `path` using custom [`WalConfig`].
    pub fn open_with_config(path: impl Into<PathBuf>, config: WalConfig) -> Result<Self> {
        let path = path.into();
        let snap_path = path.with_extension("snap");

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| io_err("create dir", parent, e))?;
            }
        }

        // Check for legacy JSON migration if WAL doesn't exist yet
        let json_path = path.with_extension("json");
        if config.migrate_legacy_json && !path.exists() && json_path.exists() {
            tracing::info!(
                legacy_json = %json_path.display(),
                wal_path = %path.display(),
                "Found legacy orders.json; initiating migration to WAL journal"
            );
            migrate_json_to_wal(&json_path, &path)?;
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| io_err("open wal", &path, e))?;

        let file_len = file
            .metadata()
            .map_err(|e| io_err("metadata", &path, e))?
            .len();
        let mut seq = 0u64;

        let crc_poly = if file_len == 0 {
            crc_poly_for_version(WAL_VERSION)
        } else {
            // Validate header and determine the CRC polynomial from the file version.
            if file_len < WAL_HEADER_SIZE as u64 {
                return Err(Mt5Error::PersistenceError(format!(
                    "'{}' is truncated or corrupt (size < 16 bytes)",
                    path.display()
                )));
            }
            let mut header = [0u8; WAL_HEADER_SIZE];
            file.seek(SeekFrom::Start(0))
                .map_err(|e| io_err("seek header", &path, e))?;
            file.read_exact(&mut header)
                .map_err(|e| io_err("read header", &path, e))?;
            if &header[..8] != WAL_MAGIC {
                return Err(Mt5Error::PersistenceError(format!(
                    "'{}' has invalid WAL magic header",
                    path.display()
                )));
            }
            let Some(version) = le_u32(&header[8..]) else {
                return Err(Mt5Error::PersistenceError(format!(
                    "'{}' has a truncated WAL version field",
                    path.display()
                )));
            };
            if version > WAL_VERSION {
                return Err(Mt5Error::PersistenceError(format!(
                    "'{}' has unsupported WAL version {version}",
                    path.display()
                )));
            }
            crc_poly_for_version(version)
        };

        if file_len == 0 {
            // Write 16-byte initial header
            let mut header = [0u8; WAL_HEADER_SIZE];
            header[..8].copy_from_slice(WAL_MAGIC);
            header[8..12].copy_from_slice(&WAL_VERSION.to_le_bytes());
            header[12..16].copy_from_slice(&1u32.to_le_bytes()); // generation = 1
            file.write_all(&header)
                .map_err(|e| io_err("write header", &path, e))?;
            file.sync_all()
                .map_err(|e| io_err("fsync header", &path, e))?;
        } else {
            // A crash can leave a partial final frame. Truncate it instead of
            // refusing to start, and never append after unreadable bytes.
            let valid_end = Self::validated_wal_end(&path, file_len)?;
            if valid_end < file_len {
                tracing::warn!(
                    wal = %path.display(),
                    discarded_bytes = file_len - valid_end,
                    "discarding torn tail of WAL"
                );
                file.set_len(valid_end)
                    .map_err(|e| io_err("truncate torn tail", &path, e))?;
                file.sync_all()
                    .map_err(|e| io_err("fsync truncated wal", &path, e))?;
            }

            // Scan the validated region to determine the current sequence.
            let mut offset = WAL_HEADER_SIZE as u64;
            while offset + FRAME_HEADER_SIZE as u64 <= valid_end {
                file.seek(SeekFrom::Start(offset))
                    .map_err(|e| io_err("seek frame", &path, e))?;
                let mut frame_hdr = [0u8; FRAME_HEADER_SIZE];
                file.read_exact(&mut frame_hdr)
                    .map_err(|e| io_err("read frame header", &path, e))?;
                let (Some(frame_seq), Some(frame_len)) =
                    (le_u64(&frame_hdr[2..]), le_u32(&frame_hdr[10..]))
                else {
                    tracing::warn!(
                        wal = %path.display(),
                        offset,
                        "stopping the WAL sequence scan at a torn frame header"
                    );
                    break;
                };
                let frame_len = frame_len as u64;
                if frame_seq > seq {
                    seq = frame_seq;
                }
                offset += FRAME_HEADER_SIZE as u64 + frame_len;
            }
        }

        file.seek(SeekFrom::End(0))
            .map_err(|e| io_err("seek end", &path, e))?;

        let inner = WalInner {
            file,
            path: path.clone(),
            snap_path,
            seq,
            entries_count: 0,
            config,
            crc_poly,
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            path,
        })
    }

    /// Path to the WAL file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append_entry(
        inner: &mut WalInner,
        record_type: u8,
        payload: &[u8],
        sync_requested: bool,
    ) -> Result<()> {
        // Record pre-write state for atomicity: on failure, truncate back
        let pre_write_seq = inner.seq;
        let pre_write_entries = inner.entries_count;
        let pre_write_offset = inner
            .file
            .stream_position()
            .map_err(|e| io_err("stream position", &inner.path, e))?;

        inner.seq += 1;
        inner.entries_count += 1;
        let seq = inner.seq;
        let len = payload.len() as u32;
        let crc = crc32(payload, inner.crc_poly);

        let mut frame_hdr = [0u8; FRAME_HEADER_SIZE];
        frame_hdr[0] = record_type;
        frame_hdr[1] = if sync_requested { FLAG_SYNC } else { 0 };
        frame_hdr[2..10].copy_from_slice(&seq.to_le_bytes());
        frame_hdr[10..14].copy_from_slice(&len.to_le_bytes());
        frame_hdr[14..18].copy_from_slice(&crc.to_le_bytes());

        // Build complete frame in one buffer for atomic write
        let mut frame = Vec::with_capacity(FRAME_HEADER_SIZE + payload.len());
        frame.extend_from_slice(&frame_hdr);
        frame.extend_from_slice(payload);

        // Write frame atomically
        let write_result = inner
            .file
            .write_all(&frame)
            .map_err(|e| io_err("write frame", &inner.path, e));

        if write_result.is_err() {
            // Truncate back to pre-write state on any write error
            inner
                .file
                .set_len(pre_write_offset)
                .map_err(|e| io_err("truncate on write failure", &inner.path, e))?;
            inner.seq = pre_write_seq;
            inner.entries_count = pre_write_entries;
            return write_result;
        }

        if sync_requested {
            inner
                .file
                .sync_data()
                .map_err(|e| io_err("fsync wal", &inner.path, e))?;
        }

        // Check if compaction is due — treat compaction as best-effort
        let file_len = inner
            .file
            .metadata()
            .map_err(|e| io_err("metadata", &inner.path, e))?
            .len();

        if inner.entries_count >= inner.config.max_wal_entries
            || file_len >= inner.config.max_wal_bytes
        {
            // Compaction failure should not fail the append; log and continue
            if let Err(e) = Self::compact_inner(inner) {
                tracing::warn!(
                    wal = %inner.path.display(),
                    error = %e,
                    "compaction failed after durable append; will retry on next append"
                );
            }
        }

        Ok(())
    }

    fn write_snapshot_file(
        snap_path: &Path,
        orders: &[TrackedOrder],
        positions: &HashMap<u64, (PositionRef, PositionState)>,
    ) -> Result<()> {
        let envelope = SnapshotEnvelope {
            format: "mt5-bridge/snapshot".to_string(),
            version: 1,
            generation: 1,
            saved_at: chrono::Utc::now().timestamp(),
            orders: orders.to_vec(),
            positions: positions.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&envelope)
            .map_err(|e| Mt5Error::PersistenceError(format!("serialize snapshot failed: {e}")))?;

        let tmp_snap = snap_path.with_extension("snap.tmp");
        {
            let mut f = File::create(&tmp_snap).map_err(|e| io_err("create", &tmp_snap, e))?;
            f.write_all(&bytes)
                .map_err(|e| io_err("write", &tmp_snap, e))?;
            f.sync_all().map_err(|e| io_err("fsync", &tmp_snap, e))?;
        }

        fs::rename(&tmp_snap, snap_path).map_err(|e| io_err("rename snapshot", snap_path, e))?;

        #[cfg(unix)]
        {
            let dir = match snap_path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            };
            let dir_file =
                File::open(&dir).map_err(|e| io_err("open snapshot directory", &dir, e))?;
            dir_file
                .sync_all()
                .map_err(|e| io_err("fsync snapshot directory", &dir, e))?;
        }

        Ok(())
    }

    fn compact_inner(inner: &mut WalInner) -> Result<()> {
        // Load current orders by replaying
        let (orders, position_states) = Self::load_from_disk(&inner.path, &inner.snap_path)?;

        // Write atomic snapshot
        Self::write_snapshot_file(&inner.snap_path, &orders, &position_states)?;

        // Reset WAL file to clean 16-byte header
        inner
            .file
            .set_len(WAL_HEADER_SIZE as u64)
            .map_err(|e| io_err("truncate wal", &inner.path, e))?;
        inner
            .file
            .seek(SeekFrom::Start(WAL_HEADER_SIZE as u64))
            .map_err(|e| io_err("seek wal header", &inner.path, e))?;
        inner
            .file
            .sync_all()
            .map_err(|e| io_err("fsync truncated wal", &inner.path, e))?;

        inner.entries_count = 0;
        tracing::debug!(
            wal = %inner.path.display(),
            retained_orders = orders.len(),
            "WAL successfully compacted into snapshot"
        );
        Ok(())
    }

    fn load_from_disk(wal_path: &Path, snap_path: &Path) -> Result<LoadResult> {
        let mut orders_map: HashMap<String, TrackedOrder> = HashMap::new();
        let mut position_states: HashMap<u64, (PositionRef, PositionState)> = HashMap::new();

        // 1. Read snapshot if it exists
        if snap_path.exists() {
            let bytes = fs::read(snap_path).map_err(|e| io_err("read snapshot", snap_path, e))?;
            let envelope: SnapshotEnvelope = serde_json::from_slice(&bytes).map_err(|e| {
                Mt5Error::PersistenceError(format!(
                    "Snapshot '{}' is corrupt: {e}. Refusing to start with corrupt state.",
                    snap_path.display()
                ))
            })?;
            for order in envelope.orders {
                orders_map.insert(order.client_order_id.clone(), order);
            }
            for (position_id, state) in envelope.positions {
                position_states.insert(position_id, state);
            }
        }

        // 2. Read WAL file entries
        if wal_path.exists() {
            let mut file = File::open(wal_path).map_err(|e| io_err("open wal", wal_path, e))?;
            let file_len = file
                .metadata()
                .map_err(|e| io_err("metadata", wal_path, e))?
                .len();

            if file_len > 0 {
                if file_len < WAL_HEADER_SIZE as u64 {
                    return Err(Mt5Error::PersistenceError(format!(
                        "WAL '{}' is corrupt (file size < 16 bytes)",
                        wal_path.display()
                    )));
                }

                let mut header = [0u8; WAL_HEADER_SIZE];
                file.read_exact(&mut header)
                    .map_err(|e| io_err("read header", wal_path, e))?;
                if &header[..8] != WAL_MAGIC {
                    return Err(Mt5Error::PersistenceError(format!(
                        "WAL '{}' has invalid magic header",
                        wal_path.display()
                    )));
                }
                let crc_poly = crc_poly_for_version(le_u32(&header[8..]).ok_or_else(|| {
                    Mt5Error::PersistenceError(format!(
                        "WAL '{}' has a truncated WAL version field",
                        wal_path.display()
                    ))
                })?);

                let mut offset = WAL_HEADER_SIZE as u64;
                while offset < file_len {
                    if offset + FRAME_HEADER_SIZE as u64 > file_len {
                        // Torn final header left by a crash mid-append.
                        tracing::warn!(
                            wal = %wal_path.display(),
                            offset,
                            "ignoring incomplete final WAL frame header"
                        );
                        break;
                    }

                    let mut frame_hdr = [0u8; FRAME_HEADER_SIZE];
                    file.read_exact(&mut frame_hdr)
                        .map_err(|e| io_err("read frame header", wal_path, e))?;

                    let rec_type = frame_hdr[0];
                    let (Some(len), Some(expected_crc)) =
                        (le_u32(&frame_hdr[10..]), le_u32(&frame_hdr[14..]))
                    else {
                        tracing::warn!(
                            wal = %wal_path.display(),
                            offset,
                            "ignoring a torn final WAL frame header"
                        );
                        break;
                    };
                    let len = len as usize;

                    offset += FRAME_HEADER_SIZE as u64;
                    if offset + len as u64 > file_len {
                        tracing::warn!(
                            wal = %wal_path.display(),
                            offset,
                            "ignoring truncated final WAL frame payload"
                        );
                        break;
                    }

                    let mut payload = vec![0u8; len];
                    file.read_exact(&mut payload)
                        .map_err(|e| io_err("read payload", wal_path, e))?;
                    offset += len as u64;

                    let actual_crc = crc32(&payload, crc_poly);
                    if actual_crc != expected_crc {
                        if offset == file_len {
                            tracing::warn!(
                                wal = %wal_path.display(),
                                offset,
                                "ignoring final WAL frame with mismatched CRC"
                            );
                            break;
                        }
                        return Err(Mt5Error::PersistenceError(format!(
                            "WAL '{}' frame CRC mismatch at offset {}: expected {expected_crc:08x}, got {actual_crc:08x}",
                            wal_path.display(),
                            offset - len as u64
                        )));
                    }

                    match rec_type {
                        REC_INTENT | REC_UPDATE => {
                            let order: TrackedOrder =
                                serde_json::from_slice(&payload).map_err(|e| {
                                    Mt5Error::PersistenceError(format!(
                                        "Failed to deserialize order in WAL '{}': {e}",
                                        wal_path.display()
                                    ))
                                })?;
                            orders_map.insert(order.client_order_id.clone(), order);
                        }
                        REC_SNAPSHOT => {
                            let orders: Vec<TrackedOrder> = serde_json::from_slice(&payload)
                                .map_err(|e| {
                                    Mt5Error::PersistenceError(format!(
                                        "Failed to deserialize snapshot in WAL '{}': {e}",
                                        wal_path.display()
                                    ))
                                })?;
                            orders_map.clear();
                            for o in orders {
                                orders_map.insert(o.client_order_id.clone(), o);
                            }
                        }
                        REC_PRUNE => {
                            let retain_ids: Vec<String> = serde_json::from_slice(&payload)
                                .map_err(|e| {
                                    Mt5Error::PersistenceError(format!(
                                        "Failed to deserialize prune list in WAL '{}': {e}",
                                        wal_path.display()
                                    ))
                                })?;
                            orders_map.retain(|k, _| retain_ids.contains(k));
                        }
                        REC_POSITION => {
                            let record: (PositionRef, PositionState) =
                                serde_json::from_slice(&payload).map_err(|e| {
                                    Mt5Error::PersistenceError(format!(
                                        "Failed to deserialize position state in WAL '{}': {e}",
                                        wal_path.display()
                                    ))
                                })?;
                            // Apply last-write-wins: later records override earlier ones
                            position_states.insert(record.0.position_id, record);
                        }
                        other => {
                            return Err(Mt5Error::PersistenceError(format!(
                                "Unknown record type {other} in WAL '{}'",
                                wal_path.display()
                            )));
                        }
                    }
                }
            }
        }

        Ok((orders_map.into_values().collect(), position_states))
    }
}

impl OrderStore for WalOrderStore {
    fn is_incremental(&self) -> bool {
        true
    }

    fn save(&self, orders: &[TrackedOrder]) -> Result<()> {
        let mut inner = self.inner.lock_recover();
        // Load current position states to preserve them in the snapshot
        let current_positions = Self::load_from_disk(&inner.path, &inner.snap_path)?.1;
        Self::write_snapshot_file(&inner.snap_path, orders, &current_positions)?;

        // Reset WAL file to 16 bytes
        inner
            .file
            .set_len(WAL_HEADER_SIZE as u64)
            .map_err(|e| io_err("truncate wal", &inner.path, e))?;
        inner
            .file
            .seek(SeekFrom::Start(WAL_HEADER_SIZE as u64))
            .map_err(|e| io_err("seek wal header", &inner.path, e))?;
        inner
            .file
            .sync_all()
            .map_err(|e| io_err("fsync wal", &inner.path, e))?;

        inner.entries_count = 0;
        Ok(())
    }

    fn load(&self) -> Result<Vec<TrackedOrder>> {
        let inner = self.inner.lock_recover();
        let (orders, _position_states) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(orders)
    }

    fn record_intent(&self, order: &TrackedOrder) -> Result<()> {
        let bytes = serde_json::to_vec(order)
            .map_err(|e| Mt5Error::PersistenceError(format!("serialize intent failed: {e}")))?;
        let mut inner = self.inner.lock_recover();
        let sync = inner.config.sync_intent;
        Self::append_entry(&mut inner, REC_INTENT, &bytes, sync)
    }

    fn record_update(&self, order: &TrackedOrder) -> Result<()> {
        let bytes = serde_json::to_vec(order)
            .map_err(|e| Mt5Error::PersistenceError(format!("serialize update failed: {e}")))?;
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_UPDATE, &bytes, false)
    }

    fn prune(&self, keep_client_order_ids: &[String]) -> Result<()> {
        let bytes = serde_json::to_vec(keep_client_order_ids)
            .map_err(|e| Mt5Error::PersistenceError(format!("serialize prune failed: {e}")))?;
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_PRUNE, &bytes, true)?;
        Self::compact_inner(&mut inner)?;
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        let inner = self.inner.lock_recover();
        inner
            .file
            .sync_data()
            .map_err(|e| io_err("flush wal", &inner.path, e))
    }
}

impl WalOrderStore {
    /// Load all persisted position-state transitions from the WAL and snapshot.
    pub fn load_position_states(&self) -> Result<HashMap<u64, (PositionRef, PositionState)>> {
        let inner = self.inner.lock_recover();
        let (_orders, position_states) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(position_states)
    }

    /// Return the highest `position_id` ever written to the WAL. Returns `None` when no position records exist.
    pub fn max_position_id(&self) -> Result<Option<u64>> {
        let inner = self.inner.lock_recover();
        let (_orders, position_states) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(position_states.keys().copied().max())
    }

    /// Persist a position-state transition (write-ahead record).
    pub fn record_position(&self, pos: &PositionRef, state: PositionState) -> Result<()> {
        let bytes = serde_json::to_vec(&(pos.clone(), state)).map_err(|e| {
            Mt5Error::PersistenceError(format!("serialize position state failed: {e}"))
        })?;
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_POSITION, &bytes, false)
    }
}

/// Migrate a legacy `orders.json` file into an append-only WAL store.
///
/// Reads the legacy JSON envelope and checksum, writes an initial snapshot file
/// (`orders.snap`), creates `orders.wal`, and renames `orders.json` to `orders.json.bak`.
pub fn migrate_json_to_wal(json_path: &Path, wal_path: &Path) -> Result<usize> {
    if !json_path.exists() {
        return Ok(0);
    }

    let json_store = JsonFileStore::new(json_path);
    let orders = json_store.load()?;
    let count = orders.len();

    let snap_path = wal_path.with_extension("snap");
    WalOrderStore::write_snapshot_file(&snap_path, &orders, &HashMap::new())?;

    // Create fresh WAL file
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(wal_path)
        .map_err(|e| io_err("create wal", wal_path, e))?;

    let mut header = [0u8; WAL_HEADER_SIZE];
    header[..8].copy_from_slice(WAL_MAGIC);
    header[8..12].copy_from_slice(&WAL_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&1u32.to_le_bytes());
    file.write_all(&header)
        .map_err(|e| io_err("write wal header", wal_path, e))?;
    file.sync_all()
        .map_err(|e| io_err("fsync wal", wal_path, e))?;

    // Rename legacy JSON to .bak
    let bak_path = json_path.with_extension("json.bak");
    fs::rename(json_path, &bak_path).map_err(|e| io_err("backup legacy json", &bak_path, e))?;

    tracing::info!(
        orders_migrated = count,
        backup = %bak_path.display(),
        "Legacy orders.json successfully migrated to WAL snapshot"
    );
    Ok(count)
}

/// Convert a WAL store back to a standalone legacy JSON file.
pub fn wal_to_json(wal_path: &Path, json_path: &Path) -> Result<usize> {
    let snap_path = wal_path.with_extension("snap");
    let (orders, _position_states) = WalOrderStore::load_from_disk(wal_path, &snap_path)?;
    let count = orders.len();

    let json_store = JsonFileStore::new(json_path);
    json_store.save(&orders)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OrderRequest, OrderType};

    fn scratch_wal(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("mt5_wal_{name}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join("orders.wal")
    }

    fn intent(cid: &str) -> TrackedOrder {
        let req = OrderRequest::buy("EURUSD", 0.5)
            .price(1.08500)
            .stop_loss(1.08000)
            .take_profit(1.09000)
            .magic(998877)
            .client_order_id(cid);
        TrackedOrder::new(&req, cid)
    }

    #[test]
    fn test_wal_roundtrip_intent_and_update() {
        let wal_path = scratch_wal("roundtrip");
        let order1 = intent("order-1");
        let mut order2 = intent("order-2");

        {
            let store = WalOrderStore::open(&wal_path).unwrap();
            assert!(store.is_incremental());
            store.record_intent(&order1).unwrap();
            store.record_intent(&order2).unwrap();

            // Transition order-2
            order2.deal_ticket = 5555;
            order2.filled_volume = 0.5;
            order2.state = crate::types::OrderState::Filled;
            store.record_update(&order2).unwrap();
        }

        // Reopen and reload
        let store = WalOrderStore::open(&wal_path).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2);

        let o1 = loaded
            .iter()
            .find(|o| o.client_order_id == "order-1")
            .unwrap();
        assert_eq!(o1.deal_ticket, 0);

        let o2 = loaded
            .iter()
            .find(|o| o.client_order_id == "order-2")
            .unwrap();
        assert_eq!(o2.deal_ticket, 5555);
        assert_eq!(o2.filled_volume, 0.5);

        drop(store);
        let _ = fs::remove_dir_all(wal_path.parent().unwrap());
    }

    #[test]
    fn test_wal_pruning_and_compaction() {
        let wal_path = scratch_wal("prune");
        let order1 = intent("order-1");
        let order2 = intent("order-2");
        let order3 = intent("order-3");

        let store = WalOrderStore::open(&wal_path).unwrap();
        store.record_intent(&order1).unwrap();
        store.record_intent(&order2).unwrap();
        store.record_intent(&order3).unwrap();

        // Prune retaining only order-2 and order-3
        store
            .prune(&["order-2".to_string(), "order-3".to_string()])
            .unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(!loaded.iter().any(|o| o.client_order_id == "order-1"));
        assert!(loaded.iter().any(|o| o.client_order_id == "order-2"));
        assert!(loaded.iter().any(|o| o.client_order_id == "order-3"));

        drop(store);
        let _ = fs::remove_dir_all(wal_path.parent().unwrap());
    }

    #[test]
    fn test_wal_position_tracking() {
        let wal_path = scratch_wal("position");
        let store = WalOrderStore::open(&wal_path).unwrap();

        let pos1 = PositionRef::new(101, "EURUSD", OrderType::Buy, 1.0, 1.0850);
        let pos2 = PositionRef::new(102, "GBPUSD", OrderType::Sell, 2.0, 1.2500);

        store.record_position(&pos1, PositionState::Open).unwrap();
        store
            .record_position(&pos2, PositionState::Closing)
            .unwrap();

        let states = store.load_position_states().unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states.get(&101).unwrap().1, PositionState::Open);
        assert_eq!(states.get(&102).unwrap().1, PositionState::Closing);
        assert_eq!(store.max_position_id().unwrap(), Some(102));

        // Update pos1 to Closed
        store.record_position(&pos1, PositionState::Closed).unwrap();
        let updated_states = store.load_position_states().unwrap();
        assert_eq!(updated_states.get(&101).unwrap().1, PositionState::Closed);

        drop(store);
        let _ = fs::remove_dir_all(wal_path.parent().unwrap());
    }

    #[test]
    fn a_file_truncated_mid_frame_header_recovers_up_to_the_last_good_frame() {
        let wal_path = scratch_wal("torn_header");
        let first = intent("torn-frame-1");
        let second = intent("torn-frame-2");

        {
            let store = WalOrderStore::open(&wal_path).unwrap();
            store.record_intent(&first).unwrap();
            store.record_intent(&second).unwrap();
        }

        // Locate the second frame and cut five bytes into its 18-byte header.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&wal_path)
            .unwrap();
        let mut wal_hdr = [0u8; WAL_HEADER_SIZE];
        file.read_exact(&mut wal_hdr).unwrap();
        let mut frame_hdr = [0u8; FRAME_HEADER_SIZE];
        file.read_exact(&mut frame_hdr).unwrap();
        let first_payload = le_u32(&frame_hdr[10..]).unwrap() as u64;
        let second_frame_start = WAL_HEADER_SIZE as u64 + FRAME_HEADER_SIZE as u64 + first_payload;
        file.set_len(second_frame_start + 5).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let store = WalOrderStore::open(&wal_path).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 1, "only the last good frame is recovered");
        assert_eq!(loaded[0].client_order_id, "torn-frame-1");

        // The torn header itself was discarded on open, not kept as data.
        let on_disk = fs::metadata(&wal_path).unwrap().len();
        assert_eq!(on_disk, second_frame_start);

        drop(store);
        let _ = fs::remove_dir_all(wal_path.parent().unwrap());
    }

    #[test]
    fn crc32_known_answer_ieee() {
        let data = b"123456789";
        assert_eq!(crc32(data, CRC_POLY_IEEE), 0xCBF4_3926);
    }

    #[test]
    fn crc32_old_polynomial_differs() {
        let data = b"123456789";
        assert_ne!(crc32(data, CRC_POLY_V2), 0xCBF4_3926);
    }

    #[test]
    fn test_corrupt_frame_in_middle_aborts_loading() {
        let wal_path = scratch_wal("corrupt_middle");
        let first = intent("order-1");
        let second = intent("order-2");
        let third = intent("order-3");

        {
            let store = WalOrderStore::open(&wal_path).unwrap();
            store.record_intent(&first).unwrap();
            store.record_intent(&second).unwrap();
            store.record_intent(&third).unwrap();
        }

        // Corrupt a byte in the payload of the first frame
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&wal_path)
            .unwrap();
        // Seek to first frame payload (offset WAL_HEADER_SIZE + FRAME_HEADER_SIZE)
        file.seek(SeekFrom::Start(
            (WAL_HEADER_SIZE + FRAME_HEADER_SIZE) as u64,
        ))
        .unwrap();
        file.write_all(&[0xFF]).unwrap();
        drop(file);

        let res = WalOrderStore::open(&wal_path);
        assert!(res.is_err(), "Middle frame corruption must be rejected");

        let _ = fs::remove_dir_all(wal_path.parent().unwrap());
    }

    #[test]
    fn test_migrate_json_to_wal_and_back() {
        let dir = std::env::temp_dir().join(format!("mt5_wal_mig_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let json_path = dir.join("orders.json");
        let wal_path = dir.join("orders.wal");

        let order1 = intent("json-order-1");
        let order2 = intent("json-order-2");
        let json_store = JsonFileStore::new(&json_path);
        json_store.save(&[order1, order2]).unwrap();

        // Migrate JSON to WAL
        let count = migrate_json_to_wal(&json_path, &wal_path).unwrap();
        assert_eq!(count, 2);
        assert!(wal_path.exists());
        assert!(dir.join("orders.snap").exists());
        assert!(dir.join("orders.json.bak").exists());

        // Read from WAL
        let wal_store = WalOrderStore::open(&wal_path).unwrap();
        let loaded = wal_store.load().unwrap();
        assert_eq!(loaded.len(), 2);

        // Convert WAL back to JSON
        let roundtrip_json = dir.join("roundtrip.json");
        let count2 = wal_to_json(&wal_path, &roundtrip_json).unwrap();
        assert_eq!(count2, 2);
        let rt_store = JsonFileStore::new(&roundtrip_json);
        assert_eq!(rt_store.load().unwrap().len(), 2);

        drop(wal_store);
        let _ = fs::remove_dir_all(&dir);
    }
}

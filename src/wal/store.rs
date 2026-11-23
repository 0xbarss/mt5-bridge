//! WAL order store implementation: [`WalConfig`], [`WalOrderStore`],
//! snapshot/compaction, load-from-disk, and the [`OrderStore`] trait impl.

use crate::error::{Mt5Error, Result};
use crate::journal::OrderStore;
use crate::position_book::{PositionRef, PositionState};
use crate::sync::LockExt;
use crate::types::{TrackedOrder, TradeRule};
use crate::wal::format::{
    crc32, crc_poly_for_version, decode_position_record, decode_string_vec, decode_tracked_order,
    decode_tracked_orders, decode_trade_rule, encode_position_record, encode_string_vec,
    encode_tracked_order, encode_trade_rule, io_err, le_u32, le_u64, read_i64, read_string,
    read_u32, read_u64, read_u8, read_vec, write_string, write_u32, write_u8, FRAME_HEADER_SIZE,
    REC_INTENT, REC_POSITION, REC_PRUNE, REC_SNAPSHOT, REC_TRADE_RULE, REC_UPDATE, WAL_HEADER_SIZE,
    WAL_MAGIC, WAL_VERSION,
};
use crate::wal::format::{LoadResult, WalConfig};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const FLAG_SYNC: u8 = 0x01;

struct SnapshotEnvelope {
    format: String,
    version: u32,
    generation: u64,
    saved_at: i64,
    orders: Vec<TrackedOrder>,
    positions: HashMap<u64, (PositionRef, PositionState)>,
}

impl SnapshotEnvelope {
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        write_string(&mut buf, &self.format).unwrap();
        buf.write_all(&self.version.to_le_bytes()).unwrap();
        buf.write_all(&self.generation.to_le_bytes()).unwrap();
        buf.write_all(&self.saved_at.to_le_bytes()).unwrap();
        write_u32(&mut buf, self.orders.len() as u32).unwrap();
        for o in &self.orders {
            o.to_bytes(&mut buf).unwrap();
        }
        write_u32(&mut buf, self.positions.len() as u32).unwrap();
        for (pos_ref, state) in self.positions.values() {
            pos_ref.to_bytes(&mut buf).unwrap();
            write_u8(&mut buf, state.to_byte()).unwrap();
        }
        buf
    }

    fn from_bytes(bytes: &[u8]) -> std::io::Result<Self> {
        let mut rd = bytes;
        let format = read_string(&mut rd)?;
        let version = read_u32(&mut rd)?;
        let generation = read_u64(&mut rd)?;
        let saved_at = read_i64(&mut rd)?;
        let orders = read_vec(&mut rd, TrackedOrder::from_bytes)?;
        let mut positions: HashMap<u64, (PositionRef, PositionState)> = HashMap::new();
        let pos_count = read_u32(&mut rd)? as usize;
        for _ in 0..pos_count {
            let pos_ref = PositionRef::from_bytes(&mut rd)?;
            let state = PositionState::from_byte(read_u8(&mut rd)?)?;
            positions.insert(pos_ref.position_id, (pos_ref, state));
        }
        Ok(Self {
            format,
            version,
            generation,
            saved_at,
            orders,
            positions,
        })
    }
}

struct WalInner {
    file: File,
    path: PathBuf,
    snap_path: PathBuf,
    seq: u64,
    entries_count: usize,
    config: WalConfig,
    crc_poly: u32,
}

/// Append-only write-ahead log (WAL) order store implementing [`OrderStore`].
#[derive(Clone)]
pub struct WalOrderStore {
    inner: Arc<Mutex<WalInner>>,
    path: PathBuf,
}

impl WalOrderStore {
    fn validated_wal_end(path: &Path, file_len: u64) -> Result<u64> {
        let mut file = File::open(path).map_err(|e| io_err("open wal", path, e))?;
        let mut offset = WAL_HEADER_SIZE as u64;

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

    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Self::open_with_config(path, WalConfig::default())
    }

    pub fn open_with_config(path: impl Into<PathBuf>, config: WalConfig) -> Result<Self> {
        let path = path.into();
        let snap_path = path.with_extension("snap");

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| io_err("create dir", parent, e))?;
            }
        }

        #[cfg(feature = "json-journal")]
        {
            let json_path = path.with_extension("json");
            if config.migrate_legacy_json && !path.exists() && json_path.exists() {
                tracing::info!(
                    legacy_json = %json_path.display(),
                    wal_path = %path.display(),
                    "Found legacy orders.json; initiating migration to WAL journal"
                );
                crate::wal::migration::migrate_json_to_wal(&json_path, &path)?;
            }
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
            let mut header = [0u8; WAL_HEADER_SIZE];
            header[..8].copy_from_slice(WAL_MAGIC);
            header[8..12].copy_from_slice(&WAL_VERSION.to_le_bytes());
            header[12..16].copy_from_slice(&1u32.to_le_bytes());
            file.write_all(&header)
                .map_err(|e| io_err("write header", &path, e))?;
            file.sync_all()
                .map_err(|e| io_err("fsync header", &path, e))?;
        } else {
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

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append_entry(
        inner: &mut WalInner,
        record_type: u8,
        payload: &[u8],
        sync_requested: bool,
    ) -> Result<()> {
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

        let mut frame = Vec::with_capacity(FRAME_HEADER_SIZE + payload.len());
        frame.extend_from_slice(&frame_hdr);
        frame.extend_from_slice(payload);

        let write_result = inner
            .file
            .write_all(&frame)
            .map_err(|e| io_err("write frame", &inner.path, e));

        if write_result.is_err() {
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

        let file_len = inner
            .file
            .metadata()
            .map_err(|e| io_err("metadata", &inner.path, e))?
            .len();

        if inner.entries_count >= inner.config.max_wal_entries
            || file_len >= inner.config.max_wal_bytes
        {
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

    pub(crate) fn write_snapshot_file(
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
        let bytes = envelope.to_bytes();

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
        let (orders, position_states, _trade_rules) =
            Self::load_from_disk(&inner.path, &inner.snap_path)?;

        Self::write_snapshot_file(&inner.snap_path, &orders, &position_states)?;

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

    pub(crate) fn load_from_disk(wal_path: &Path, snap_path: &Path) -> Result<LoadResult> {
        let mut orders_map: HashMap<String, TrackedOrder> = HashMap::new();
        let mut position_states: HashMap<u64, (PositionRef, PositionState)> = HashMap::new();
        let mut trade_rules: Vec<TradeRule> = Vec::new();

        if snap_path.exists() {
            let bytes = fs::read(snap_path).map_err(|e| io_err("read snapshot", snap_path, e))?;
            let envelope = SnapshotEnvelope::from_bytes(&bytes).map_err(|e| {
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
                            let order = decode_tracked_order(&payload).map_err(|e| {
                                Mt5Error::PersistenceError(format!(
                                    "Failed to deserialize order in WAL '{}': {e}",
                                    wal_path.display()
                                ))
                            })?;
                            orders_map.insert(order.client_order_id.clone(), order);
                        }
                        REC_SNAPSHOT => {
                            let orders = decode_tracked_orders(&payload).map_err(|e| {
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
                            let retain_ids = decode_string_vec(&payload).map_err(|e| {
                                Mt5Error::PersistenceError(format!(
                                    "Failed to deserialize prune list in WAL '{}': {e}",
                                    wal_path.display()
                                ))
                            })?;
                            orders_map.retain(|k, _| retain_ids.contains(k));
                        }
                        REC_POSITION => {
                            let record = decode_position_record(&payload).map_err(|e| {
                                Mt5Error::PersistenceError(format!(
                                    "Failed to deserialize position state in WAL '{}': {e}",
                                    wal_path.display()
                                ))
                            })?;
                            position_states.insert(record.0.position_id, record);
                        }
                        REC_TRADE_RULE => {
                            let rule = decode_trade_rule(&payload).map_err(|e| {
                                Mt5Error::PersistenceError(format!(
                                    "Failed to deserialize trade rule in WAL '{}': {e}",
                                    wal_path.display()
                                ))
                            })?;
                            trade_rules.retain(|r| r.client_order_id != rule.client_order_id);
                            trade_rules.push(rule);
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

        Ok((
            orders_map.into_values().collect(),
            position_states,
            trade_rules,
        ))
    }
}

impl OrderStore for WalOrderStore {
    fn is_incremental(&self) -> bool {
        true
    }

    fn save(&self, orders: &[TrackedOrder]) -> Result<()> {
        let mut inner = self.inner.lock_recover();
        let current_positions = Self::load_from_disk(&inner.path, &inner.snap_path)?.1;
        Self::write_snapshot_file(&inner.snap_path, orders, &current_positions)?;

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
        let (orders, _, _) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(orders)
    }

    fn record_intent(&self, order: &TrackedOrder) -> Result<()> {
        let bytes = encode_tracked_order(order);
        let mut inner = self.inner.lock_recover();
        let sync = inner.config.sync_intent;
        Self::append_entry(&mut inner, REC_INTENT, &bytes, sync)
    }

    fn record_update(&self, order: &TrackedOrder) -> Result<()> {
        let bytes = encode_tracked_order(order);
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_UPDATE, &bytes, false)
    }

    fn prune(&self, keep_client_order_ids: &[String]) -> Result<()> {
        let bytes = encode_string_vec(keep_client_order_ids);
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
    pub fn load_position_states(&self) -> Result<HashMap<u64, (PositionRef, PositionState)>> {
        let inner = self.inner.lock_recover();
        let (_, position_states, _) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(position_states)
    }

    pub fn max_position_id(&self) -> Result<Option<u64>> {
        let inner = self.inner.lock_recover();
        let (_, position_states, _) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(position_states.keys().copied().max())
    }

    pub fn record_position(&self, pos: &PositionRef, state: PositionState) -> Result<()> {
        let bytes = encode_position_record(pos, &state);
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_POSITION, &bytes, false)
    }

    pub fn record_trade_rule(&self, _symbol: &str, rule: &TradeRule) -> Result<()> {
        let bytes = encode_trade_rule(rule);
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_TRADE_RULE, &bytes, false)
    }

    pub fn load_trade_rules(&self, symbol: &str) -> Result<Vec<TradeRule>> {
        let inner = self.inner.lock_recover();
        let (_orders, _position_states, trade_rules) =
            Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(trade_rules
            .into_iter()
            .filter(|r| r.symbol == symbol)
            .collect())
    }
}

//! Durable write-ahead log (WAL) order store.
//!
//! Provides high-throughput, low-latency append-only order journaling.
//! Pre-send intents (`OrderState::Submitting`) are written as small binary-framed
//! entries with synchronous `fsync`, while subsequent state transitions (e.g. fills,
//! modifications, closures) are appended incrementally without rewriting the entire state.
//!
//! Periodic atomic snapshots (`orders.snap`) and pruning keep WAL files compact and bounded.

use crate::error::{Mt5Error, Result};
use crate::journal::OrderStore;
use crate::position_book::{PositionRef, PositionState};
use crate::sync::LockExt;
use crate::types::TrackedOrder;
use std::collections::{BTreeMap, HashMap};
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
const REC_TRADE_RULE: u8 = 0x06;

/// Return type of [`WalOrderStore::load_from_disk`]: orders, the persisted
/// position-state transitions, and trade-management rules needed to rebuild
/// bracket/trailing state after restart.
type LoadResult = (
    Vec<TrackedOrder>,
    HashMap<u64, (PositionRef, PositionState)>,
    Vec<TradeRule>,
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

/// Binary serialization helpers for WAL payloads.
///
/// The WAL frame format is binary; `serde_json` is only needed for the legacy
/// `JsonFileStore` migration path. These helpers encode the journaled types
/// (orders, fills, positions, snapshot envelope) to/from little-endian bytes
/// so `WalOrderStore` compiles and works with `--no-default-features --features wal`.
use crate::types::{
    AttributeMismatch, Fill, MismatchKind, OrderState, OrderType, RejectionEvidence, TradeRule,
};

// ----- primitives -----

fn write_u8<W: Write>(w: &mut W, v: u8) -> std::io::Result<()> {
    w.write_all(&[v])
}
fn write_u32<W: Write>(w: &mut W, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_u64<W: Write>(w: &mut W, v: u64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_i32<W: Write>(w: &mut W, v: i32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_i64<W: Write>(w: &mut W, v: i64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_f64<W: Write>(w: &mut W, v: f64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn read_u8<R: Read>(r: &mut R) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
fn read_u32<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_u64<R: Read>(r: &mut R) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn read_i32<R: Read>(r: &mut R) -> std::io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}
fn read_i64<R: Read>(r: &mut R) -> std::io::Result<i64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(i64::from_le_bytes(b))
}
fn read_f64<R: Read>(r: &mut R) -> std::io::Result<f64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(f64::from_le_bytes(b))
}

/// Write a length-prefixed UTF-8 string.
fn write_string<W: Write>(w: &mut W, s: &str) -> std::io::Result<()> {
    write_u32(w, s.len() as u32)?;
    w.write_all(s.as_bytes())
}
/// Read a length-prefixed UTF-8 string.
fn read_string<R: Read>(r: &mut R) -> std::io::Result<String> {
    let len = read_u32(r)? as usize;
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    String::from_utf8(b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write an `Option<T>` as a discriminant byte plus the value when `Some`.
fn write_option<W: Write, F: FnOnce(&mut W, &T) -> std::io::Result<()>, T>(
    w: &mut W,
    opt: &Option<T>,
    f: F,
) -> std::io::Result<()> {
    match opt {
        Some(v) => {
            write_u8(w, 1)?;
            f(w, v)
        }
        None => write_u8(w, 0),
    }
}
/// Read an `Option<T>`.
fn read_option<R: Read, F: FnOnce(&mut R) -> std::io::Result<T>, T>(
    r: &mut R,
    f: F,
) -> std::io::Result<Option<T>> {
    match read_u8(r)? {
        0 => Ok(None),
        1 => f(r).map(Some),
        other => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid option discriminant {other}"),
        )),
    }
}

/// Write a length-prefixed vector.
fn write_vec<W: Write, F: FnMut(&mut W, &T) -> std::io::Result<()>, T>(
    w: &mut W,
    items: &[T],
    mut f: F,
) -> std::io::Result<()> {
    write_u32(w, items.len() as u32)?;
    for item in items {
        f(w, item)?;
    }
    Ok(())
}
/// Read a length-prefixed vector.
fn read_vec<R: Read, F: FnMut(&mut R) -> std::io::Result<T>, T>(
    r: &mut R,
    mut f: F,
) -> std::io::Result<Vec<T>> {
    let len = read_u32(r)? as usize;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        out.push(f(r)?);
    }
    Ok(out)
}

// ----- enums -----

impl OrderType {
    fn to_byte(self) -> u8 {
        match self {
            OrderType::Buy => 0,
            OrderType::Sell => 1,
            OrderType::BuyLimit => 2,
            OrderType::SellLimit => 3,
            OrderType::BuyStop => 4,
            OrderType::SellStop => 5,
        }
    }
    fn from_byte(b: u8) -> std::io::Result<Self> {
        Ok(match b {
            0 => OrderType::Buy,
            1 => OrderType::Sell,
            2 => OrderType::BuyLimit,
            3 => OrderType::SellLimit,
            4 => OrderType::BuyStop,
            5 => OrderType::SellStop,
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid OrderType discriminant {other}"),
                ))
            }
        })
    }
}

impl OrderState {
    fn to_byte(self) -> u8 {
        match self {
            OrderState::Created => 0,
            OrderState::Submitting => 1,
            OrderState::Accepted => 2,
            OrderState::PartiallyFilled => 3,
            OrderState::Filled => 4,
            OrderState::Cancelled => 5,
            OrderState::Rejected => 6,
            OrderState::Unknown => 7,
            OrderState::Reconciled => 8,
            OrderState::Mismatched => 9,
        }
    }
    fn from_byte(b: u8) -> std::io::Result<Self> {
        Ok(match b {
            0 => OrderState::Created,
            1 => OrderState::Submitting,
            2 => OrderState::Accepted,
            3 => OrderState::PartiallyFilled,
            4 => OrderState::Filled,
            5 => OrderState::Cancelled,
            6 => OrderState::Rejected,
            7 => OrderState::Unknown,
            8 => OrderState::Reconciled,
            9 => OrderState::Mismatched,
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid OrderState discriminant {other}"),
                ))
            }
        })
    }
}

impl MismatchKind {
    fn to_byte(self) -> u8 {
        match self {
            MismatchKind::Symbol => 0,
            MismatchKind::Direction => 1,
            MismatchKind::OrderType => 2,
            MismatchKind::Volume => 3,
            MismatchKind::Price => 4,
            MismatchKind::StopLoss => 5,
            MismatchKind::TakeProfit => 6,
            MismatchKind::Magic => 7,
        }
    }
    fn from_byte(b: u8) -> std::io::Result<Self> {
        Ok(match b {
            0 => MismatchKind::Symbol,
            1 => MismatchKind::Direction,
            2 => MismatchKind::OrderType,
            3 => MismatchKind::Volume,
            4 => MismatchKind::Price,
            5 => MismatchKind::StopLoss,
            6 => MismatchKind::TakeProfit,
            7 => MismatchKind::Magic,
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid MismatchKind discriminant {other}"),
                ))
            }
        })
    }
}

impl PositionState {
    fn to_byte(self) -> u8 {
        match self {
            PositionState::Pending => 0,
            PositionState::Open => 1,
            PositionState::Closing => 2,
            PositionState::Closed => 3,
            PositionState::Unknown => 4,
        }
    }
    fn from_byte(b: u8) -> std::io::Result<Self> {
        Ok(match b {
            0 => PositionState::Pending,
            1 => PositionState::Open,
            2 => PositionState::Closing,
            3 => PositionState::Closed,
            4 => PositionState::Unknown,
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid PositionState discriminant {other}"),
                ))
            }
        })
    }
}

impl RejectionEvidence {
    fn to_byte(&self) -> u8 {
        match self {
            RejectionEvidence::NeverTransmitted => 0,
            RejectionEvidence::PreflightRejected => 1,
            RejectionEvidence::BrokerRetcode(_) => 2,
            RejectionEvidence::ReconciledAbsent { .. } => 3,
            RejectionEvidence::OperatorResolved { .. } => 4,
        }
    }
    fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        write_u8(w, self.to_byte())?;
        match self {
            RejectionEvidence::NeverTransmitted => {}
            RejectionEvidence::PreflightRejected => {}
            RejectionEvidence::BrokerRetcode(code) => write_i32(w, *code)?,
            RejectionEvidence::ReconciledAbsent {
                observations,
                span_secs,
            } => {
                write_u32(w, *observations)?;
                write_i64(w, *span_secs)?;
            }
            RejectionEvidence::OperatorResolved { note } => write_string(w, note)?,
        }
        Ok(())
    }
    fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
        let tag = read_u8(r)?;
        Ok(match tag {
            0 => RejectionEvidence::NeverTransmitted,
            1 => RejectionEvidence::PreflightRejected,
            2 => RejectionEvidence::BrokerRetcode(read_i32(r)?),
            3 => RejectionEvidence::ReconciledAbsent {
                observations: read_u32(r)?,
                span_secs: read_i64(r)?,
            },
            4 => RejectionEvidence::OperatorResolved {
                note: read_string(r)?,
            },
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid RejectionEvidence discriminant {other}"),
                ))
            }
        })
    }
}

// ----- leaf types -----

impl Fill {
    fn to_bytes<W: Write>(self, w: &mut W) -> std::io::Result<()> {
        write_u64(w, self.deal)?;
        write_f64(w, self.volume)?;
        write_f64(w, self.price)?;
        write_i64(w, self.time)?;
        write_i32(w, self.deal_type)?;
        write_i32(w, self.deal_entry)?;
        Ok(())
    }
    fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            deal: read_u64(r)?,
            volume: read_f64(r)?,
            price: read_f64(r)?,
            time: read_i64(r)?,
            deal_type: read_i32(r)?,
            deal_entry: read_i32(r)?,
        })
    }
}

impl AttributeMismatch {
    fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        write_u8(w, self.kind.to_byte())?;
        write_string(w, &self.expected)?;
        write_string(w, &self.actual)?;
        Ok(())
    }
    fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            kind: MismatchKind::from_byte(read_u8(r)?)?,
            expected: read_string(r)?,
            actual: read_string(r)?,
        })
    }
}

impl PositionRef {
    fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        write_u64(w, self.position_id)?;
        write_string(w, &self.symbol)?;
        write_option(w, &self.ticket, |w, v| write_u64(w, *v))?;
        write_string(w, &self.client_order_id)?;
        write_u8(w, self.direction.to_byte())?;
        write_f64(w, self.volume)?;
        write_f64(w, self.entry_price)?;
        write_u64(w, self.magic)?;
        Ok(())
    }
    fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            position_id: read_u64(r)?,
            symbol: read_string(r)?,
            ticket: read_option(r, read_u64)?,
            client_order_id: read_string(r)?,
            direction: OrderType::from_byte(read_u8(r)?)?,
            volume: read_f64(r)?,
            entry_price: read_f64(r)?,
            magic: read_u64(r)?,
        })
    }
}

// ----- TradeRule -----

impl TradeRule {
    /// Binary-serialize this rule into `w`.
    fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        write_string(w, &self.client_order_id)?;
        write_string(w, &self.symbol)?;
        write_u64(w, self.ticket)?;
        write_u8(w, self.order_type.to_byte())?;
        write_f64(w, self.entry_price)?;
        write_f64(w, self.volume)?;
        write_f64(w, self.stop_loss)?;
        write_f64(w, self.take_profit)?;
        write_option(w, &self.break_even_at_r, |w, v| write_f64(w, *v))?;
        write_option(w, &self.trailing_distance, |w, v| write_f64(w, *v))?;
        write_option(w, &self.trailing_step, |w, v| write_f64(w, *v))?;
        write_option(w, &self.trailing_activation, |w, v| write_f64(w, *v))?;
        write_f64(w, self.risk)?;
        write_f64(w, self.stops_level)?;
        write_i64(w, self.last_modify_secs)?;
        write_u8(w, u8::from(self.break_even_activated))?;
        write_u8(w, u8::from(self.trailing_activated))?;
        Ok(())
    }

    /// Deserialize a `TradeRule` from binary bytes.
    fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            client_order_id: read_string(r)?,
            symbol: read_string(r)?,
            ticket: read_u64(r)?,
            order_type: OrderType::from_byte(read_u8(r)?)?,
            entry_price: read_f64(r)?,
            volume: read_f64(r)?,
            stop_loss: read_f64(r)?,
            take_profit: read_f64(r)?,
            break_even_at_r: read_option(r, read_f64)?,
            trailing_distance: read_option(r, read_f64)?,
            trailing_step: read_option(r, read_f64)?,
            trailing_activation: read_option(r, read_f64)?,
            risk: read_f64(r)?,
            stops_level: read_f64(r)?,
            last_modify_secs: read_i64(r)?,
            break_even_activated: read_u8(r)? != 0,
            trailing_activated: read_u8(r)? != 0,
        })
    }
}

/// Convenience wrappers usable from the parent module for WAL frame payloads.
fn encode_trade_rule(rule: &TradeRule) -> Vec<u8> {
    let mut buf = Vec::new();
    rule.to_bytes(&mut buf).expect("infallible write to Vec");
    buf
}
fn decode_trade_rule(bytes: &[u8]) -> std::io::Result<TradeRule> {
    TradeRule::from_bytes(&mut &bytes[..])
}

// ----- TrackedOrder -----

impl TrackedOrder {
    /// Binary-serialize this order into `w`.
    fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        write_string(w, &self.client_order_id)?;
        write_string(w, &self.wire_id)?;
        write_string(w, &self.symbol)?;
        write_u8(w, self.order_type.to_byte())?;
        write_f64(w, self.requested_volume)?;
        write_f64(w, self.requested_price)?;
        write_f64(w, self.requested_stop_loss)?;
        write_f64(w, self.requested_take_profit)?;
        write_f64(w, self.filled_volume)?;
        write_f64(w, self.remaining_volume)?;
        write_f64(w, self.average_price)?;
        write_u64(w, self.order_ticket)?;
        write_u64(w, self.deal_ticket)?;
        write_vec(w, &self.deal_tickets, |w, v| write_u64(w, *v))?;
        write_u64(w, self.position_ticket)?;
        write_u8(w, self.state.to_byte())?;
        write_u64(w, self.magic)?;
        write_i64(w, self.created_at)?;
        write_i64(w, self.updated_at)?;
        write_option(w, &self.error_message, |w, v| write_string(w, v))?;
        write_u32(w, self.absent_observations)?;
        write_option(w, &self.first_absent_at, |w, v| write_i64(w, *v))?;
        write_vec(w, &self.mismatches, |w, v| v.to_bytes(w))?;
        write_option(w, &self.rejection, |w, v| v.to_bytes(w))?;
        write_u32(w, self.fills.len() as u32)?;
        for (k, v) in &self.fills {
            write_u64(w, *k)?;
            v.to_bytes(w)?;
        }
        Ok(())
    }

    /// Deserialize a `TrackedOrder` from binary bytes.
    fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            client_order_id: read_string(r)?,
            wire_id: read_string(r)?,
            symbol: read_string(r)?,
            order_type: OrderType::from_byte(read_u8(r)?)?,
            requested_volume: read_f64(r)?,
            requested_price: read_f64(r)?,
            requested_stop_loss: read_f64(r)?,
            requested_take_profit: read_f64(r)?,
            filled_volume: read_f64(r)?,
            remaining_volume: read_f64(r)?,
            average_price: read_f64(r)?,
            order_ticket: read_u64(r)?,
            deal_ticket: read_u64(r)?,
            deal_tickets: read_vec(r, read_u64)?,
            position_ticket: read_u64(r)?,
            state: OrderState::from_byte(read_u8(r)?)?,
            magic: read_u64(r)?,
            created_at: read_i64(r)?,
            updated_at: read_i64(r)?,
            error_message: read_option(r, read_string)?,
            absent_observations: read_u32(r)?,
            first_absent_at: read_option(r, read_i64)?,
            mismatches: read_vec(r, AttributeMismatch::from_bytes)?,
            rejection: read_option(r, RejectionEvidence::from_bytes)?,
            fills: {
                let fill_count = read_u32(r)? as usize;
                let mut fills = BTreeMap::new();
                for _ in 0..fill_count {
                    let k = read_u64(r)?;
                    let v = Fill::from_bytes(r)?;
                    fills.insert(k, v);
                }
                fills
            },
        })
    }
}

// Convenience wrappers usable from the parent module for WAL frame payloads.
fn encode_tracked_order(order: &TrackedOrder) -> Vec<u8> {
    let mut buf = Vec::new();
    order.to_bytes(&mut buf).expect("infallible write to Vec");
    buf
}
fn decode_tracked_order(bytes: &[u8]) -> std::io::Result<TrackedOrder> {
    TrackedOrder::from_bytes(&mut &bytes[..])
}
fn decode_tracked_orders(bytes: &[u8]) -> std::io::Result<Vec<TrackedOrder>> {
    read_vec(&mut &bytes[..], TrackedOrder::from_bytes)
}
fn encode_string_vec(items: &[String]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_vec(&mut buf, items, |w, s| write_string(w, s)).expect("infallible write to Vec");
    buf
}
fn decode_string_vec(bytes: &[u8]) -> std::io::Result<Vec<String>> {
    read_vec(&mut &bytes[..], read_string)
}
fn encode_position_record(pos: &PositionRef, state: &PositionState) -> Vec<u8> {
    let mut buf = Vec::new();
    pos.to_bytes(&mut buf).expect("infallible write to Vec");
    write_u8(&mut buf, state.to_byte()).expect("infallible write to Vec");
    buf
}
fn decode_position_record(bytes: &[u8]) -> std::io::Result<(PositionRef, PositionState)> {
    let mut r = bytes;
    Ok((
        PositionRef::from_bytes(&mut r)?,
        PositionState::from_byte(read_u8(&mut r)?)?,
    ))
}

// ----- end of binary_serialization helpers -----

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

/// Header written at the start of every snapshot file.
struct SnapshotEnvelope {
    format: String,
    version: u32,
    generation: u64,
    saved_at: i64,
    orders: Vec<TrackedOrder>,
    /// Persisted position-state transitions. All states are written; on load
    /// we apply last-write-wins and drop terminal states (Closed).
    positions: HashMap<u64, (PositionRef, PositionState)>,
}

impl SnapshotEnvelope {
    /// Serialize the envelope to a self-describing binary buffer (no `serde_json`).
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        write_string(&mut buf, &self.format).unwrap();
        buf.write_all(&self.version.to_le_bytes()).unwrap();
        buf.write_all(&self.generation.to_le_bytes()).unwrap();
        buf.write_all(&self.saved_at.to_le_bytes()).unwrap();
        write_vec(&mut buf, &self.orders, |buf, o| o.to_bytes(buf)).unwrap();
        write_u32(&mut buf, self.positions.len() as u32).unwrap();
        for (pos_ref, state) in self.positions.values() {
            pos_ref.to_bytes(&mut buf).unwrap();
            write_u8(&mut buf, state.to_byte()).unwrap();
        }
        buf
    }

    /// Deserialize an envelope from binary bytes. Returns `Err` on truncation or corruption.
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
        #[cfg(feature = "json-journal")]
        {
            let json_path = path.with_extension("json");
            if config.migrate_legacy_json && !path.exists() && json_path.exists() {
                tracing::info!(
                    legacy_json = %json_path.display(),
                    wal_path = %path.display(),
                    "Found legacy orders.json; initiating migration to WAL journal"
                );
                migrate_json_to_wal(&json_path, &path)?;
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
        // Load current orders by replaying
        let (orders, position_states, _trade_rules) =
            Self::load_from_disk(&inner.path, &inner.snap_path)?;

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
        let mut trade_rules: Vec<TradeRule> = Vec::new();

        // 1. Read snapshot if it exists
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
                            // Apply last-write-wins: later records override earlier ones
                            position_states.insert(record.0.position_id, record);
                        }
                        REC_TRADE_RULE => {
                            let rule = decode_trade_rule(&payload).map_err(|e| {
                                Mt5Error::PersistenceError(format!(
                                    "Failed to deserialize trade rule in WAL '{}': {e}",
                                    wal_path.display()
                                ))
                            })?;
                            // Last-write-wins: replace any existing rule for this client order ID
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
    /// Load all persisted position-state transitions from the WAL and snapshot.
    pub fn load_position_states(&self) -> Result<HashMap<u64, (PositionRef, PositionState)>> {
        let inner = self.inner.lock_recover();
        let (_, position_states, _) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(position_states)
    }

    /// Return the highest `position_id` ever written to the WAL. Returns `None` when no position records exist.
    pub fn max_position_id(&self) -> Result<Option<u64>> {
        let inner = self.inner.lock_recover();
        let (_, position_states, _) = Self::load_from_disk(&inner.path, &inner.snap_path)?;
        Ok(position_states.keys().copied().max())
    }

    /// Persist a position-state transition (write-ahead record).
    pub fn record_position(&self, pos: &PositionRef, state: PositionState) -> Result<()> {
        let bytes = encode_position_record(pos, &state);
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_POSITION, &bytes, false)
    }

    /// Persist a trade-management rule to the WAL as a `REC_TRADE_RULE` frame.
    pub fn record_trade_rule(&self, _symbol: &str, rule: &TradeRule) -> Result<()> {
        let bytes = encode_trade_rule(rule);
        let mut inner = self.inner.lock_recover();
        Self::append_entry(&mut inner, REC_TRADE_RULE, &bytes, false)
    }

    /// Load all persisted trade rules from the WAL and snapshot, optionally filtered by symbol.
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
#[cfg(feature = "json-journal")]
pub fn wal_to_json(wal_path: &Path, json_path: &Path) -> Result<usize> {
    let snap_path = wal_path.with_extension("snap");
    let (orders, _, _) = WalOrderStore::load_from_disk(wal_path, &snap_path)?;
    let count = orders.len();

    let json_store = crate::journal::JsonFileStore::new(json_path);
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

    #[cfg(feature = "json-journal")]
    #[test]
    fn test_migrate_json_to_wal_and_back() {
        let dir = std::env::temp_dir().join(format!("mt5_wal_mig_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let json_path = dir.join("orders.json");
        let wal_path = dir.join("orders.wal");

        let order1 = intent("json-order-1");
        let order2 = intent("json-order-2");
        let json_store = crate::journal::JsonFileStore::new(&json_path);
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
        let rt_store = crate::journal::JsonFileStore::new(&roundtrip_json);
        assert_eq!(rt_store.load().unwrap().len(), 2);

        drop(wal_store);
        let _ = fs::remove_dir_all(&dir);
    }
}

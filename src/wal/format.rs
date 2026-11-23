//! Binary frame format, CRC, and read/write primitives for the WAL layer.
//!
//! Encodes journaled types (orders, fills, positions, snapshot envelope,
//! trade rules) to/from little-endian bytes so `WalOrderStore` compiles and
//! works with `--no-default-features --features wal`.

use crate::error::Mt5Error;
use crate::position_book::{PositionRef, PositionState};
use crate::types::{
    AttributeMismatch, Fill, MismatchKind, OrderState, OrderType, RejectionEvidence, TrackedOrder,
    TradeRule,
};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::Path;

pub const WAL_MAGIC: &[u8; 8] = b"MT5WAL01";
pub const WAL_VERSION: u32 = 3;
pub const WAL_HEADER_SIZE: usize = 16;
pub const FRAME_HEADER_SIZE: usize = 18;

pub const REC_INTENT: u8 = 0x01;
pub const REC_UPDATE: u8 = 0x02;
pub const REC_SNAPSHOT: u8 = 0x03;
pub const REC_PRUNE: u8 = 0x04;
pub const REC_POSITION: u8 = 0x05;
pub const REC_TRADE_RULE: u8 = 0x06;

pub const FLAG_SYNC: u8 = 0x01;

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

pub const CRC_POLY_IEEE: u32 = 0xEDB8_8320;
pub const CRC_POLY_V2: u32 = 0xED88_8320;

/// Return the CRC polynomial for a given WAL file version.
pub fn crc_poly_for_version(version: u32) -> u32 {
    match version {
        2 => CRC_POLY_V2,
        _ => CRC_POLY_IEEE,
    }
}

// ----- primitives -----

pub(crate) fn write_u8<W: Write>(w: &mut W, v: u8) -> std::io::Result<()> {
    w.write_all(&[v])
}
pub(crate) fn write_u32<W: Write>(w: &mut W, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
pub(crate) fn write_u64<W: Write>(w: &mut W, v: u64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
pub(crate) fn write_i32<W: Write>(w: &mut W, v: i32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
pub(crate) fn write_i64<W: Write>(w: &mut W, v: i64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
pub(crate) fn write_f64<W: Write>(w: &mut W, v: f64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

pub(crate) fn read_u8<R: Read>(r: &mut R) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
pub(crate) fn read_u32<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
pub(crate) fn read_u64<R: Read>(r: &mut R) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
pub(crate) fn read_i32<R: Read>(r: &mut R) -> std::io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}
pub(crate) fn read_i64<R: Read>(r: &mut R) -> std::io::Result<i64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(i64::from_le_bytes(b))
}
pub(crate) fn read_f64<R: Read>(r: &mut R) -> std::io::Result<f64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(f64::from_le_bytes(b))
}

/// Write a length-prefixed UTF-8 string.
pub(crate) fn write_string<W: Write>(w: &mut W, s: &str) -> std::io::Result<()> {
    write_u32(w, s.len() as u32)?;
    w.write_all(s.as_bytes())
}
/// Read a length-prefixed UTF-8 string.
pub(crate) fn read_string<R: Read>(r: &mut R) -> std::io::Result<String> {
    let len = read_u32(r)? as usize;
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    String::from_utf8(b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write an `Option<T>` as a discriminant byte plus the value when `Some`.
pub(crate) fn write_option<W: Write, F: FnOnce(&mut W, &T) -> std::io::Result<()>, T>(
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
pub(crate) fn read_option<R: Read, F: FnOnce(&mut R) -> std::io::Result<T>, T>(
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
pub(crate) fn write_vec<W: Write, F: FnMut(&mut W, &T) -> std::io::Result<()>, T>(
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
pub(crate) fn read_vec<R: Read, F: FnMut(&mut R) -> std::io::Result<T>, T>(
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

/// Little-endian `u32` out of the first four bytes of `b`.
pub fn le_u32(b: &[u8]) -> Option<u32> {
    b.get(..4)?.try_into().ok().map(u32::from_le_bytes)
}

/// Little-endian `u64` out of the first eight bytes of `b`.
pub fn le_u64(b: &[u8]) -> Option<u64> {
    b.get(..8)?.try_into().ok().map(u64::from_le_bytes)
}

// ----- enums -----

impl OrderType {
    pub(crate) fn to_byte(self) -> u8 {
        match self {
            OrderType::Buy => 0,
            OrderType::Sell => 1,
            OrderType::BuyLimit => 2,
            OrderType::SellLimit => 3,
            OrderType::BuyStop => 4,
            OrderType::SellStop => 5,
        }
    }
    pub(crate) fn from_byte(b: u8) -> std::io::Result<Self> {
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
    pub(crate) fn to_byte(self) -> u8 {
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
    pub(crate) fn from_byte(b: u8) -> std::io::Result<Self> {
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
    pub(crate) fn to_byte(self) -> u8 {
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
    pub(crate) fn from_byte(b: u8) -> std::io::Result<Self> {
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
    pub(crate) fn to_byte(self) -> u8 {
        match self {
            PositionState::Pending => 0,
            PositionState::Open => 1,
            PositionState::Closing => 2,
            PositionState::Closed => 3,
            PositionState::Unknown => 4,
        }
    }
    pub(crate) fn from_byte(b: u8) -> std::io::Result<Self> {
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
    pub(crate) fn to_byte(&self) -> u8 {
        match self {
            RejectionEvidence::NeverTransmitted => 0,
            RejectionEvidence::PreflightRejected => 1,
            RejectionEvidence::BrokerRetcode(_) => 2,
            RejectionEvidence::ReconciledAbsent { .. } => 3,
            RejectionEvidence::OperatorResolved { .. } => 4,
        }
    }
    pub(crate) fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
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
    pub(crate) fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
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
    pub(crate) fn to_bytes<W: Write>(self, w: &mut W) -> std::io::Result<()> {
        write_u64(w, self.deal)?;
        write_f64(w, self.volume)?;
        write_f64(w, self.price)?;
        write_i64(w, self.time)?;
        write_i32(w, self.deal_type)?;
        write_i32(w, self.deal_entry)?;
        Ok(())
    }
    pub(crate) fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
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
    pub(crate) fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        write_u8(w, self.kind.to_byte())?;
        write_string(w, &self.expected)?;
        write_string(w, &self.actual)?;
        Ok(())
    }
    pub(crate) fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            kind: MismatchKind::from_byte(read_u8(r)?)?,
            expected: read_string(r)?,
            actual: read_string(r)?,
        })
    }
}

impl PositionRef {
    pub(crate) fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
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
    pub(crate) fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
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
    pub(crate) fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
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
    pub(crate) fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
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

// ----- TrackedOrder -----

impl TrackedOrder {
    /// Binary-serialize this order into `w`.
    pub(crate) fn to_bytes<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
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
    pub(crate) fn from_bytes<R: Read>(r: &mut R) -> std::io::Result<Self> {
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

// ----- convenience wrappers -----

pub(crate) fn encode_trade_rule(rule: &TradeRule) -> Vec<u8> {
    let mut buf = Vec::new();
    rule.to_bytes(&mut buf).expect("infallible write to Vec");
    buf
}
pub(crate) fn decode_trade_rule(bytes: &[u8]) -> std::io::Result<TradeRule> {
    TradeRule::from_bytes(&mut &bytes[..])
}

pub(crate) fn encode_tracked_order(order: &TrackedOrder) -> Vec<u8> {
    let mut buf = Vec::new();
    order.to_bytes(&mut buf).expect("infallible write to Vec");
    buf
}
pub(crate) fn decode_tracked_order(bytes: &[u8]) -> std::io::Result<TrackedOrder> {
    TrackedOrder::from_bytes(&mut &bytes[..])
}
pub(crate) fn decode_tracked_orders(bytes: &[u8]) -> std::io::Result<Vec<TrackedOrder>> {
    read_vec(&mut &bytes[..], TrackedOrder::from_bytes)
}
pub(crate) fn encode_string_vec(items: &[String]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_vec(&mut buf, items, |w, s| write_string(w, s)).expect("infallible write to Vec");
    buf
}
pub(crate) fn decode_string_vec(bytes: &[u8]) -> std::io::Result<Vec<String>> {
    read_vec(&mut &bytes[..], read_string)
}
pub(crate) fn encode_position_record(pos: &PositionRef, state: &PositionState) -> Vec<u8> {
    let mut buf = Vec::new();
    pos.to_bytes(&mut buf).expect("infallible write to Vec");
    write_u8(&mut buf, state.to_byte()).expect("infallible write to Vec");
    buf
}
pub(crate) fn decode_position_record(
    bytes: &[u8],
) -> std::io::Result<(PositionRef, PositionState)> {
    let mut r = bytes;
    Ok((
        PositionRef::from_bytes(&mut r)?,
        PositionState::from_byte(read_u8(&mut r)?)?,
    ))
}

/// Return type of [`WalOrderStore::load_from_disk`]: orders, the persisted
/// position-state transitions, and trade-management rules needed to rebuild
/// bracket/trailing state after restart.
pub type LoadResult = (
    Vec<crate::types::TrackedOrder>,
    HashMap<u64, (PositionRef, PositionState)>,
    Vec<TradeRule>,
);

pub(crate) fn io_err(what: &str, path: &Path, e: std::io::Error) -> Mt5Error {
    Mt5Error::PersistenceError(format!("{what} '{}': {e}", path.display()))
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

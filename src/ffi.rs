//! Low-level C FFI bindings and packed struct layouts matching `mt5_bridge.h`.
//!
//! All structs are marked `#[repr(C, packed)]` to match the `#pragma pack(push, 1)`
//! packing in the C/C++ DLL and MQL5 serialiser. When the `schema` feature is
//! enabled, they additionally derive `schemars::JsonSchema` for JSON Schema export.

use static_assertions::{assert_eq_size, const_assert_eq};
use std::os::raw::{c_char, c_double, c_int};

/// Wire format for symbol specification (76 bytes).
///
/// Matches the C++ side, which asserts `sizeof(Mt5SymInfo) == 76` in
/// `mt5_bridge.cpp`. Fields: seven `f64` price/lot fields, one `i32` digits,
/// then `bid` and `ask`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mt5SymInfo {
    pub point: f64,
    pub tick_value: f64,
    pub tick_size: f64,
    pub lot_step: f64,
    pub min_lot: f64,
    pub max_lot: f64,
    pub spread: f64,
    pub digits: i32,
    /// The last bid the terminal reported.
    pub bid: f64,
    /// The last ask the terminal reported.
    pub ask: f64,
}

/// Wire format for OHLCV bar rate (60 bytes).
/// Corresponds directly to MT5 `MqlRates`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mt5Rate {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub spread: i32,
    pub _rv: i64, // real_volume
}

/// Wire format for price tick (44 bytes).
/// Corresponds directly to MT5 `MqlTick`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mt5Tick {
    pub time: i64,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume: u64,
    pub flags: u32,
}

/// Wire format for order execution results (44 bytes).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mt5TradeResult {
    pub retcode: u32,
    pub deal: u64,
    pub order: u64,
    pub position: u64,
    pub volume: f64,
    pub price: f64,
}

/// Wire format for open position (148 bytes).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5Position {
    pub ticket: u64,
    pub time: i64,
    pub position_type: i32, // 0 = Buy, 1 = Sell
    pub magic: u64,
    pub volume: f64,
    pub price_open: f64,
    pub sl: f64,
    pub tp: f64,
    pub price_current: f64,
    pub profit: f64,
    pub swap: f64,
    pub symbol: [u8; 32],
    pub comment: [u8; 32],
}

impl Default for Mt5Position {
    fn default() -> Self {
        Self {
            ticket: 0,
            time: 0,
            position_type: 0,
            magic: 0,
            volume: 0.0,
            price_open: 0.0,
            sl: 0.0,
            tp: 0.0,
            price_current: 0.0,
            profit: 0.0,
            swap: 0.0,
            symbol: [0u8; 32],
            comment: [0u8; 32],
        }
    }
}

/// Wire format for working pending order (140 bytes).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5Order {
    pub ticket: u64,
    pub time_setup: i64,
    pub order_type: i32, // 2 = BuyLimit, 3 = SellLimit, 4 = BuyStop, 5 = SellStop
    pub magic: u64,
    pub volume_initial: f64,
    pub volume_current: f64,
    pub price_open: f64,
    pub sl: f64,
    pub tp: f64,
    pub price_current: f64,
    pub symbol: [u8; 32],
    pub comment: [u8; 32],
}

impl Default for Mt5Order {
    fn default() -> Self {
        Self {
            ticket: 0,
            time_setup: 0,
            order_type: 0,
            magic: 0,
            volume_initial: 0.0,
            volume_current: 0.0,
            price_open: 0.0,
            sl: 0.0,
            tp: 0.0,
            price_current: 0.0,
            symbol: [0u8; 32],
            comment: [0u8; 32],
        }
    }
}

/// Wire format for a completed deal from MT5 history (152 bytes).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5Deal {
    pub ticket: u64,
    pub order: u64,
    pub position_id: u64,
    pub time: i64,
    pub deal_type: i32, // 0 = Buy, 1 = Sell (non-trade deals are filtered out by the EA)
    pub entry: i32,     // 0 = In, 1 = Out, 2 = InOut, 3 = OutBy
    pub magic: u64,
    pub volume: f64,
    pub price: f64,
    pub commission: f64,
    pub swap: f64,
    pub profit: f64,
    pub symbol: [u8; 32],
    pub comment: [u8; 32],
}

impl Default for Mt5Deal {
    fn default() -> Self {
        Self {
            ticket: 0,
            order: 0,
            position_id: 0,
            time: 0,
            deal_type: 0,
            entry: 0,
            magic: 0,
            volume: 0.0,
            price: 0.0,
            commission: 0.0,
            swap: 0.0,
            profit: 0.0,
            symbol: [0u8; 32],
            comment: [0u8; 32],
        }
    }
}

/// Wire format for packet header (16 bytes, protocol v6+).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHdr {
    pub length: u32,
    pub kind: u8,
    pub _pad: u8,
    pub id: u16,
    pub status: i32,
    pub request_id: u32,
}

/// Asynchronous market data tick event pushed by the EA (88 bytes, protocol v6+).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5TickEvent {
    pub symbol: [u8; 32],
    pub time_msc: i64,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume: u64,
    pub flags: u32,
    pub _pad: i32,
    pub event_seq: u64,
}

impl Default for Mt5TickEvent {
    fn default() -> Self {
        Self {
            symbol: [0u8; 32],
            time_msc: 0,
            bid: 0.0,
            ask: 0.0,
            last: 0.0,
            volume: 0,
            flags: 0,
            _pad: 0,
            event_seq: 0,
        }
    }
}

/// Asynchronous trade transaction event pushed by the EA (176 bytes, protocol v6+).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5TradeEvent {
    pub deal: u64,
    pub order: u64,
    pub position: u64,
    pub time: i64,
    pub trans_type: i32,
    pub order_type: i32,
    pub price: f64,
    pub volume: f64,
    pub sl: f64,
    pub tp: f64,
    pub symbol: [u8; 32],
    pub comment: [u8; 32],
    pub deal_entry: i32,
    pub deal_type: i32,
    pub deal_magic: u64,
    pub deal_position_id: u64,
    pub deal_reason: i32,
    pub _pad: i32,
    pub event_seq: u64,
}

impl Default for Mt5TradeEvent {
    fn default() -> Self {
        Self {
            deal: 0,
            order: 0,
            position: 0,
            time: 0,
            trans_type: 0,
            order_type: 0,
            price: 0.0,
            volume: 0.0,
            sl: 0.0,
            tp: 0.0,
            symbol: [0u8; 32],
            comment: [0u8; 32],
            deal_entry: 0,
            deal_type: 0,
            deal_magic: 0,
            deal_position_id: 0,
            deal_reason: 0,
            _pad: 0,
            event_seq: 0,
        }
    }
}

/// Asynchronous depth-of-market book event pushed by the EA (72 bytes, protocol v6+).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5BookEvent {
    pub symbol: [u8; 32],
    pub time_msc: i64,
    pub book_type: i32,
    pub _pad: i32,
    pub price: f64,
    pub volume: f64,
    pub event_seq: u64,
}

impl Default for Mt5BookEvent {
    fn default() -> Self {
        Self {
            symbol: [0u8; 32],
            time_msc: 0,
            book_type: 0,
            _pad: 0,
            price: 0.0,
            volume: 0.0,
            event_seq: 0,
        }
    }
}

/// Asynchronous bar event pushed by the EA upon bar close (96 bytes, protocol v7+).
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5BarEvent {
    pub symbol: [u8; 32],
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub timeframe: i32,
    pub spread: i32,
    pub event_seq: u64,
}

impl Default for Mt5BarEvent {
    fn default() -> Self {
        Self {
            symbol: [0u8; 32],
            time: 0,
            open: 0.0,
            high: 0.0,
            low: 0.0,
            close: 0.0,
            volume: 0,
            timeframe: 0,
            spread: 0,
            event_seq: 0,
        }
    }
}

/// Bridge-wide statistics snapshot (64 bytes, packed).
///
/// Returned by `CMD_STATS` (27). Mirrors the `Mt5Stats` struct in
/// `bridge_dll/mt5_bridge.h`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mt5Stats {
    pub uptime_secs: u64,
    pub requests_total: u64,
    pub tick_drops: u64,
    pub trade_drops: u64,
    pub book_drops: u64,
    pub bar_drops: u64,
    pub tick_queue_depth: u32,
    pub trade_queue_depth: u32,
    pub last_error: i32,
    pub protocol: u32,
}

/// Packet kind on the wire (protocol v5+).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketKind {
    Request = 0,
    Response = 1,
    Event = 2,
}

/// Event kind on the wire (protocol v5+).
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Tick = 1,
    Trade = 2,
    Book = 3,
    Bar = 4,
}

/// Wire transaction kinds (WTX_*) matching EA and C++ DLL
pub const WTX_OTHER: i32 = 0;
pub const WTX_DEAL_ADD: i32 = 1;
pub const WTX_ORDER_ADD: i32 = 2;
pub const WTX_ORDER_DELETE: i32 = 3;
pub const WTX_ORDER_UPDATE: i32 = 4;
pub const WTX_POSITION: i32 = 5;
pub const WTX_REQUEST: i32 = 6;

/// Frame and query caps
pub const MT5_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const MT5_MAX_BARS_PER_FETCH: usize =
    (MT5_MAX_FRAME_BYTES - 64) / std::mem::size_of::<Mt5Rate>();

// Wire protocol handshake version matching mt5_bridge.h and mt5_bridge.mq5.
pub const PROTOCOL_VERSION: u32 = 8;

pub const MT5_OK: i32 = 1;
pub const MT5_ERR_GENERAL: i32 = 0;
pub const MT5_ERR_SEND_FAILED: i32 = -1;
pub const MT5_ERR_UNKNOWN_EXECUTION: i32 = -2;
pub const MT5_ERR_PIPE_DISCONNECTED: i32 = -3;
pub const MT5_ERR_SERVER_AUTH_FAILED: i32 = -4;
pub const MT5_ERR_EA_REJECTED: i32 = -5;
pub const MT5_ERR_ALREADY_INITIALIZED: i32 = -6;
pub const MT5_ERR_BUFFER_TOO_SMALL: i32 = -7;

/// Largest of four sizes, usable in const contexts (MSRV-compatible; `Ord::max`
/// is not yet const-stable).
const fn max_event_size(a: usize, b: usize, c: usize, d: usize) -> usize {
    let m = if a > b { a } else { b };
    let m = if m > c { m } else { c };
    if m > d {
        m
    } else {
        d
    }
}

/// Maximum wire payload size of any asynchronous event pushed by the EA.
///
/// Sized from the largest event struct at compile time so the Rust event pump
/// can allocate a buffer that fits every event type (`Mt5TradeEvent` = 176 bytes
/// today) and never has to parse a truncated payload. Kept in sync with the
/// packed-struct size assertions below.
pub const MAX_EVENT_SIZE: usize = max_event_size(
    std::mem::size_of::<Mt5TradeEvent>(),
    std::mem::size_of::<Mt5TickEvent>(),
    std::mem::size_of::<Mt5BookEvent>(),
    std::mem::size_of::<Mt5BarEvent>(),
);

// Ensure struct memory layouts match C header at compile-time.
const_assert_eq!(std::mem::size_of::<Mt5SymInfo>(), 76);
const_assert_eq!(std::mem::size_of::<Mt5Rate>(), 60);
const_assert_eq!(std::mem::size_of::<Mt5Tick>(), 44);
const_assert_eq!(std::mem::size_of::<Mt5TradeResult>(), 44);
const_assert_eq!(std::mem::size_of::<Mt5Position>(), 148);
const_assert_eq!(std::mem::size_of::<Mt5Order>(), 140);
const_assert_eq!(std::mem::size_of::<Mt5Deal>(), 152);
const_assert_eq!(std::mem::size_of::<PacketHdr>(), 16);
const_assert_eq!(std::mem::size_of::<Mt5TickEvent>(), 88);
const_assert_eq!(std::mem::size_of::<Mt5TradeEvent>(), 176);
const_assert_eq!(std::mem::size_of::<Mt5BookEvent>(), 72);
const_assert_eq!(std::mem::size_of::<Mt5BarEvent>(), 96);
const_assert_eq!(std::mem::size_of::<Mt5Stats>(), 64);

// The pump buffer is allocated from MAX_EVENT_SIZE; this pins the documented
// 176-byte value so any future event layout change is noticed here.
const_assert_eq!(MAX_EVENT_SIZE, 176);

// Ensure field offsets match expected layout (validates packing).
assert_eq_size!(Mt5SymInfo, [u8; 76]);
assert_eq_size!(Mt5Rate, [u8; 60]);
assert_eq_size!(Mt5Tick, [u8; 44]);
assert_eq_size!(Mt5TradeResult, [u8; 44]);
assert_eq_size!(Mt5Position, [u8; 148]);
assert_eq_size!(Mt5Order, [u8; 140]);
assert_eq_size!(Mt5Deal, [u8; 152]);
assert_eq_size!(PacketHdr, [u8; 16]);
assert_eq_size!(Mt5TickEvent, [u8; 88]);
assert_eq_size!(Mt5TradeEvent, [u8; 176]);
assert_eq_size!(Mt5BookEvent, [u8; 72]);
assert_eq_size!(Mt5BarEvent, [u8; 96]);
assert_eq_size!(Mt5Stats, [u8; 64]);

// Function pointer signatures for dynamic library loading.
pub type FnInit = unsafe extern "C" fn(i64, *const c_char, *const c_char) -> c_int;
pub type FnShut = unsafe extern "C" fn() -> c_int;
pub type FnIsConnected = unsafe extern "C" fn() -> c_int;
pub type FnRates =
    unsafe extern "C" fn(*const c_char, c_int, i64, i64, *mut Mt5Rate, c_int) -> c_int;
pub type FnAcct = unsafe extern "C" fn(
    *mut f64,
    *mut f64,
    *mut f64,
    *mut f64,
    *mut f64,
    *mut c_char,
    c_int,
) -> c_int;
pub type FnSend = unsafe extern "C" fn(
    *const c_char,
    c_int,
    c_double,
    c_double,
    c_double,
    c_double,
    *const c_char,
    u32,
    i64,
    u64,
    *mut Mt5TradeResult,
) -> c_int;
pub type FnClose = unsafe extern "C" fn(u64, *mut Mt5TradeResult) -> c_int;
pub type FnCloseMagic = unsafe extern "C" fn(u64, u64, *mut Mt5TradeResult) -> c_int;
pub type FnCloseDeviation =
    unsafe extern "C" fn(u64, u64, u32, c_double, *mut Mt5TradeResult) -> c_int;
pub type FnModify =
    unsafe extern "C" fn(u64, u8, c_double, u8, c_double, *mut Mt5TradeResult) -> c_int;
pub type FnModifyMagic =
    unsafe extern "C" fn(u64, u64, u8, c_double, u8, c_double, *mut Mt5TradeResult) -> c_int;
pub type FnSymTick = unsafe extern "C" fn(*const c_char, *mut Mt5Tick) -> c_int;
pub type FnSymInfo = unsafe extern "C" fn(*const c_char, *mut Mt5SymInfo) -> c_int;
pub type FnPositions = unsafe extern "C" fn(*mut Mt5Position, c_int, u64, *const c_char) -> c_int;
pub type FnOrders = unsafe extern "C" fn(*mut Mt5Order, c_int, u64, *const c_char) -> c_int;
pub type FnDeals = unsafe extern "C" fn(*mut Mt5Deal, c_int, i64, i64, u64, *const c_char) -> c_int;

// Protocol subscription and event function signatures.
pub type FnSubscribeTicks = unsafe extern "C" fn(*const c_char) -> c_int;
pub type FnUnsubscribeTicks = unsafe extern "C" fn(*const c_char) -> c_int;
pub type FnSubscribeTrade = unsafe extern "C" fn() -> c_int;
pub type FnUnsubscribeTrade = unsafe extern "C" fn() -> c_int;
pub type FnSubscribeBook = unsafe extern "C" fn(*const c_char) -> c_int;
pub type FnUnsubscribeBook = unsafe extern "C" fn(*const c_char) -> c_int;
pub type FnSubscribeBars = unsafe extern "C" fn(*const c_char, c_int) -> c_int;
pub type FnUnsubscribeBars = unsafe extern "C" fn(*const c_char, c_int) -> c_int;
pub type FnPing = unsafe extern "C" fn(*mut i64, *mut u64, *mut u32) -> c_int;
pub type FnSymbolsTotal = unsafe extern "C" fn() -> c_int;
pub type FnSymbolsGet = unsafe extern "C" fn(u32, u32, *mut c_char, c_int) -> c_int;
pub type FnSymbolSelect = unsafe extern "C" fn(*const c_char, c_int) -> c_int;
pub type FnCalendarEvents = unsafe extern "C" fn(u64, u64, *mut c_char, c_int) -> c_int;
pub type FnCalendarCountries = unsafe extern "C" fn(*mut c_char, c_int) -> c_int;
pub type Mt5EventCallback = unsafe extern "C" fn(u16, *const u8, u32);
pub type FnRegisterEventCallback = unsafe extern "C" fn(Mt5EventCallback) -> c_int;
pub type FnPollEvent = unsafe extern "C" fn(*mut u16, *mut u8, u32, *mut u32, u32) -> c_int;
pub type FnEventsDroppedTotal = unsafe extern "C" fn() -> u64;
pub type FnGetOrphanCount = unsafe extern "C" fn() -> u64;
pub type FnStats = unsafe extern "C" fn(*mut c_char, c_int) -> c_int;

/// Protocol command IDs defined in `mt5_bridge.cpp` and `mt5_bridge.mq5`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolCmd {
    Init = 1,
    Shutdown = 2,
    Rates = 3,
    Account = 4,
    OrderSend = 5,
    OrderClose = 6,
    OrderModify = 7,
    SymTick = 8,
    SymInfo = 9,
    PositionsGet = 10,
    OrdersGet = 11,
    DealsGet = 12,
    SubscribeTicks = 13,
    UnsubscribeTicks = 14,
    SubscribeTrade = 15,
    UnsubscribeTrade = 16,
    SubscribeBook = 17,
    UnsubscribeBook = 18,
    SubscribeBars = 19,
    UnsubscribeBars = 20,
    Ping = 21,
    SymbolsTotal = 22,
    SymbolsGet = 23,
    SymbolSelect = 24,
    CalendarEvents = 25,
    CalendarCountries = 26,
    Stats = 27,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_event_size_covers_every_event_payload() {
        let sizes = [
            std::mem::size_of::<Mt5TickEvent>(),
            std::mem::size_of::<Mt5TradeEvent>(),
            std::mem::size_of::<Mt5BookEvent>(),
            std::mem::size_of::<Mt5BarEvent>(),
        ];
        assert_eq!(MAX_EVENT_SIZE, 176);
        assert_eq!(MAX_EVENT_SIZE, *sizes.iter().max().unwrap());
        assert!(sizes.iter().all(|&size| size <= MAX_EVENT_SIZE));
    }

    #[test]
    fn mt5_err_buffer_too_small_is_a_distinct_negative_code() {
        assert_eq!(MT5_ERR_BUFFER_TOO_SMALL, -7);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_OK);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_ERR_GENERAL);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_ERR_SEND_FAILED);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_ERR_UNKNOWN_EXECUTION);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_ERR_PIPE_DISCONNECTED);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_ERR_SERVER_AUTH_FAILED);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_ERR_EA_REJECTED);
        assert_ne!(MT5_ERR_BUFFER_TOO_SMALL, MT5_ERR_ALREADY_INITIALIZED);
    }
}

//! Low-level C FFI bindings and packed struct layouts matching `mt5_bridge.h`.
//!
//! All structs are marked `#[repr(C, packed)]` to match the `#pragma pack(push, 1)`
//! packing in the C/C++ DLL and MQL5 serialiser.

use std::os::raw::{c_char, c_double, c_int};

/// Wire format for symbol specification (60 bytes).
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
}

/// Wire format for OHLCV bar rate (60 bytes).
/// Corresponds directly to MT5 `MqlRates`.
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

/// Wire format for packet header (12 bytes, protocol v5+).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHdr {
    pub length: u32,
    pub kind: u8,
    pub _pad: u8,
    pub id: u16,
    pub status: i32,
}

/// Asynchronous market data tick event pushed by the EA (76 bytes, protocol v5+).
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
        }
    }
}

/// Asynchronous trade transaction event pushed by the EA (136 bytes, protocol v5+).
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
        }
    }
}

/// Asynchronous depth-of-market book event pushed by the EA (64 bytes, protocol v5+).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mt5BookEvent {
    pub symbol: [u8; 32],
    pub time_msc: i64,
    pub book_type: i32,
    pub _pad: i32,
    pub price: f64,
    pub volume: f64,
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
        }
    }
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

// Wire protocol handshake version matching mt5_bridge.h and mt5_bridge.mq5.
//
// v5: push model, asynchronous EVENT packets (TICK, TRADE, BOOK), subscriptions,
//     and full-duplex named pipe streaming.
pub const PROTOCOL_VERSION: u32 = 5;

pub const MT5_OK: i32 = 1;
pub const MT5_ERR_GENERAL: i32 = 0;
pub const MT5_ERR_SEND_FAILED: i32 = -1;
pub const MT5_ERR_UNKNOWN_EXECUTION: i32 = -2;
pub const MT5_ERR_PIPE_DISCONNECTED: i32 = -3;

// Ensure struct memory layouts match C header at compile-time.
const _: () = {
    assert!(std::mem::size_of::<Mt5SymInfo>() == 60);
    assert!(std::mem::size_of::<Mt5Rate>() == 60);
    assert!(std::mem::size_of::<Mt5Tick>() == 44);
    assert!(std::mem::size_of::<Mt5TradeResult>() == 44);
    assert!(std::mem::size_of::<Mt5Position>() == 148);
    assert!(std::mem::size_of::<Mt5Order>() == 140);
    assert!(std::mem::size_of::<Mt5Deal>() == 152);
    assert!(std::mem::size_of::<PacketHdr>() == 12);
    assert!(std::mem::size_of::<Mt5TickEvent>() == 76);
    assert!(std::mem::size_of::<Mt5TradeEvent>() == 136);
    assert!(std::mem::size_of::<Mt5BookEvent>() == 64);
};

// Function pointer signatures for dynamic library loading.
pub type FnInit = unsafe extern "C" fn(i64, *const c_char, *const c_char) -> c_int;
pub type FnShut = unsafe extern "C" fn() -> c_int;
pub type FnRates =
    unsafe extern "C" fn(*const c_char, c_int, i64, i64, *mut Mt5Rate, c_int) -> c_int;
pub type FnAcct = unsafe extern "C" fn(*mut f64, *mut f64, *mut f64, *mut f64) -> c_int;
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
pub type FnModify = unsafe extern "C" fn(u64, c_double, c_double, *mut Mt5TradeResult) -> c_int;
pub type FnModifyMagic =
    unsafe extern "C" fn(u64, u64, c_double, c_double, *mut Mt5TradeResult) -> c_int;
pub type FnSymTick = unsafe extern "C" fn(*const c_char, *mut Mt5Tick) -> c_int;
pub type FnSymInfo = unsafe extern "C" fn(*const c_char, *mut Mt5SymInfo) -> c_int;
pub type FnPositions = unsafe extern "C" fn(*mut Mt5Position, c_int, u64, *const c_char) -> c_int;
pub type FnOrders = unsafe extern "C" fn(*mut Mt5Order, c_int, u64, *const c_char) -> c_int;
pub type FnDeals = unsafe extern "C" fn(*mut Mt5Deal, c_int, i64, i64, u64, *const c_char) -> c_int;

// Protocol v5 subscription and event function signatures.
pub type FnSubscribeTicks = unsafe extern "C" fn(*const c_char) -> c_int;
pub type FnUnsubscribeTicks = unsafe extern "C" fn(*const c_char) -> c_int;
pub type FnSubscribeTrade = unsafe extern "C" fn() -> c_int;
pub type FnUnsubscribeTrade = unsafe extern "C" fn() -> c_int;
pub type FnSubscribeBook = unsafe extern "C" fn(*const c_char) -> c_int;
pub type FnUnsubscribeBook = unsafe extern "C" fn(*const c_char) -> c_int;
pub type Mt5EventCallback = unsafe extern "C" fn(u16, *const u8, u32);
pub type FnRegisterEventCallback = unsafe extern "C" fn(Mt5EventCallback) -> c_int;
pub type FnPollEvent = unsafe extern "C" fn(*mut u16, *mut u8, u32, *mut u32, u32) -> c_int;
pub type FnEventsDroppedTotal = unsafe extern "C" fn() -> u64;

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
}

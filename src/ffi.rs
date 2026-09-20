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

// Wire protocol handshake version matching mt5_bridge.h and mt5_bridge.mq5.
//
// v4: adds `CMD_DEALS_GET` / `Mt5Deal` (trade-history query) and changes the order comment
//     wire format from `cid:<truncated raw id>` to `cid:<13-char hash token>`. A v3 EA would
//     mis-handle both, so the handshake refuses to pair mismatched halves.
pub const PROTOCOL_VERSION: u32 = 4;

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
pub type FnPositions =
    unsafe extern "C" fn(*mut Mt5Position, c_int, u64, *const c_char) -> c_int;
pub type FnOrders = unsafe extern "C" fn(*mut Mt5Order, c_int, u64, *const c_char) -> c_int;
pub type FnDeals =
    unsafe extern "C" fn(*mut Mt5Deal, c_int, i64, i64, u64, *const c_char) -> c_int;

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
}

//! Low-level C FFI bindings and packed struct layouts matching `mt5_bridge.h`.
//!
//! All structs are marked `#[repr(C, packed)]` to match the `#pragma pack(push, 1)`
//! packing in the C/C++ DLL and MQL5 serialiser.

use std::os::raw::{c_char, c_double, c_int, c_long};

/// Wire format for symbol specification (52 bytes).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mt5SymInfo {
    pub point: f64,
    pub tick_value: f64,
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

/// Wire format for order execution results (36 bytes).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mt5TradeResult {
    pub retcode: u32,
    pub deal: u64,
    pub order: u64,
    pub volume: f64,
    pub price: f64,
}

// Ensure struct memory layouts match C header at compile-time.
const _: () = {
    assert!(std::mem::size_of::<Mt5SymInfo>() == 52);
    assert!(std::mem::size_of::<Mt5Rate>() == 60);
    assert!(std::mem::size_of::<Mt5Tick>() == 44);
    assert!(std::mem::size_of::<Mt5TradeResult>() == 36);
};

// Function pointer signatures for dynamic library loading.
pub type FnInit = unsafe extern "C" fn(c_long, *const c_char, *const c_char) -> c_int;
pub type FnShut = unsafe extern "C" fn() -> c_int;
pub type FnRates = unsafe extern "C" fn(*const c_char, c_int, i64, i64, *mut Mt5Rate) -> c_int;
pub type FnAcct = unsafe extern "C" fn(*mut f64, *mut f64, *mut f64, *mut f64) -> c_int;
pub type FnSend = unsafe extern "C" fn(
    *const c_char,
    c_int,
    c_double,
    c_double,
    c_double,
    c_double,
    *const c_char,
    *mut Mt5TradeResult,
) -> c_int;
pub type FnClose = unsafe extern "C" fn(u64, *mut Mt5TradeResult) -> c_int;
pub type FnModify = unsafe extern "C" fn(u64, c_double, c_double) -> c_int;
pub type FnSymTick = unsafe extern "C" fn(*const c_char, *mut Mt5Tick) -> c_int;
pub type FnSymInfo = unsafe extern "C" fn(*const c_char, *mut Mt5SymInfo) -> c_int;

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
}

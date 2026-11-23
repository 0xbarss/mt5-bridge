#![no_main]
//! Fuzz the wire decode of `Mt5TradeResult` (44-byte packed execution result).
//! Exercises `TradeResult::from_raw` with attacker-controlled packed bytes
//! (including NaN/Inf floats and out-of-enum discriminants).

use libfuzzer_sys::fuzz_target;
use mt5_bridge::ffi::Mt5TradeResult;
use mt5_bridge::types::TradeResult;

mod common;

fuzz_target!(|bytes: &[u8]| {
    if let Some(raw) = common::read_packed::<Mt5TradeResult>(bytes) {
        // `from_raw` only copies fields and uses `from_utf8_lossy`; it must not
        // panic on any bit pattern.
        let _ = TradeResult::from_raw(raw);
    }
});

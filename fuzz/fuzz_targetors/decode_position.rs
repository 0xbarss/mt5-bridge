#![no_main]
//! Fuzz the wire decode of `Mt5Position` (148-byte packed open position).
//! Exercises `Position::from_raw` with attacker-controlled packed bytes.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::ffi::Mt5Position;
use mt5_bridge::types::Position;

mod common;

fuzz_target!(|bytes: &[u8]| {
    if let Some(raw) = common::read_packed::<Mt5Position>(bytes) {
        let _ = Position::from_raw(raw);
    }
});

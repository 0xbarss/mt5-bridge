#![no_main]
//! Fuzz the wire decode of `Mt5Deal` (152-byte packed completed deal).
//! Exercises `Deal::from_raw` with attacker-controlled packed bytes.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::ffi::Mt5Deal;
use mt5_bridge::types::Deal;

mod common;

fuzz_target!(|bytes: &[u8]| {
    if let Some(raw) = common::read_packed::<Mt5Deal>(bytes) {
        let _ = Deal::from_raw(raw);
    }
});

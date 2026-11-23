#![no_main]
//! Fuzz the wire decode of `Mt5BarEvent` (96-byte packed bar event).
//! Exercises `Bar::from_event` with attacker-controlled packed bytes.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::ffi::Mt5BarEvent;
use mt5_bridge::types::Bar;

mod common;

fuzz_target!(|bytes: &[u8]| {
    if let Some(raw) = common::read_packed::<Mt5BarEvent>(bytes) {
        let _ = Bar::from_event(&raw);
    }
});

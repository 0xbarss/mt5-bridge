#![no_main]
//! Fuzz the wire decode of `Mt5TickEvent` (88-byte packed tick event).
//! Exercises `Tick::from_event` with attacker-controlled packed bytes.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::ffi::Mt5TickEvent;
use mt5_bridge::types::Tick;

mod common;

fuzz_target!(|bytes: &[u8]| {
    if let Some(raw) = common::read_packed::<Mt5TickEvent>(bytes) {
        let _ = Tick::from_event(raw);
    }
});

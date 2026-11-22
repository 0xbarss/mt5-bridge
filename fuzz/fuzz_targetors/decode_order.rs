#![no_main]
//! Fuzz the wire decode of `Mt5Order` (140-byte packed working pending order).
//! Exercises `WorkingOrder::from_raw` with attacker-controlled packed bytes.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::ffi::Mt5Order;
use mt5_bridge::types::WorkingOrder;

mod common;

fuzz_target!(|bytes: &[u8]| {
    if let Some(raw) = common::read_packed::<Mt5Order>(bytes) {
        let _ = WorkingOrder::from_raw(raw);
    }
});

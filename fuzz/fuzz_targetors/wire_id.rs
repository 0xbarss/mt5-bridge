#![no_main]
//! Fuzz `wire_id` (the Crockford-base32 wire token derivation) with
//! attacker-controlled client order IDs. The hash + render pipeline must not
//! panic on any valid UTF-8 input.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::wire_id;

fuzz_target!(|s: String| {
    let _ = wire_id(&s);
});

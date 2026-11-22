#![no_main]
//! Fuzz `parse_wire_id` with attacker-controlled comment strings.
//! The extractor must never panic on malformed or oversized input.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::parse_wire_id;

fuzz_target!(|s: String| {
    let _ = parse_wire_id(&s);
});

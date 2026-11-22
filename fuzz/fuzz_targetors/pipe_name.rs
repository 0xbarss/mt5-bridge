#![no_main]
//! Fuzz `validate_pipe_name` with attacker-controlled pipe names.
//! The validator must reject malformed names without panicking.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::client::validate_pipe_name;

fuzz_target!(|s: String| {
    let _ = validate_pipe_name(&s);
});

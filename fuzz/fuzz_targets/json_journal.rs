#![no_main]
//! Fuzz the JSON journal (`JsonFileStore`) parse path by writing attacker-controlled
//! bytes to a temp file and loading them. This exercises `serde_json` deserialisation
//! of `TrackedOrder` plus the envelope format/version/checksum validation.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::journal::JsonFileStore;
use mt5_bridge::OrderStore;

fn fuzz_journal_path() -> std::path::PathBuf {
    std::env::temp_dir().join("mt5_bridge_fuzz_json_journal.json")
}

fuzz_target!(|bytes: &[u8]| {
    let path = fuzz_journal_path();
    // Overwrite the temp journal with arbitrary input on every run.
    let _ = std::fs::write(&path, bytes);
    let store = JsonFileStore::new(&path);
    // `load` must return an error on corrupt input, never panic.
    let _ = store.load();
    // Best-effort cleanup; ignore failures (the file is overwritten next run).
    let _ = std::fs::remove_file(&path);
});

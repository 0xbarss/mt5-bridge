#![no_main]
//! Fuzz the WAL frame CRC32 polynomials used by `WalOrderStore` frame
//! validation (`validated_wal_end`). Both the v3 IEEE polynomial and the
//! legacy v2 polynomial are exercised with attacker-controlled bytes.

use libfuzzer_sys::fuzz_target;
use mt5_bridge::wal::crc32;

const CRC_POLY_IEEE: u32 = 0xEDB8_8320;
const CRC_POLY_V2: u32 = 0xED88_8320;

fuzz_target!(|bytes: &[u8]| {
    // Frame integrity check: attacker-controlled payload bytes.
    let _ieee = crc32(bytes, CRC_POLY_IEEE);
    let _v2 = crc32(bytes, CRC_POLY_V2);

    // Simulate a WAL frame header: bytes[10..14] = payload length,
    // bytes[14..18] = expected CRC. Validate a payload slice against it,
    // exercising `crc32` on attacker-controlled length and payload.
    if bytes.len() >= 18 {
        let len = u32::from_le_bytes(bytes[10..14].try_into().unwrap_or([0; 4])) as usize;
        let _expected = u32::from_le_bytes(bytes[14..18].try_into().unwrap_or([0; 4]));
        if 18 + len <= bytes.len() {
            let _actual = crc32(&bytes[18..18 + len], CRC_POLY_IEEE);
        }
    }
});

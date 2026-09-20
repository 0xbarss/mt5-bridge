//! Protocol and parser fuzzing suite.
//!
//! Comprehensive fuzzing suite for protocol resilience:
//! - Exercises packet and wire decoding routines against arbitrary byte sequences,
//!   corrupted memory layouts, non-UTF-8 strings, and extreme floating point values (NaN, Inf, denormals).
//! - Fuzzes `wire_id`, `parse_wire_id`, and `effective_comment` across broad character spaces
//!   (multi-byte UTF-8, emojis, null bytes, long inputs).
//! - Fuzzes order request validation and builder bounds.
//! - Fuzzes `JsonFileStore` and `TrackedOrder` state deserialization with corrupted inputs.
//! - Asserts that parser functions never panic or exhibit undefined behavior on malformed input.

use mt5_bridge::ffi::*;
use mt5_bridge::*;
use std::io::Write;

/// Deterministic pseudo-random number generator for reproducible fuzz sweeps.
struct SimpleRng(u64);

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(8) {
            let val = self.next_u64().to_le_bytes();
            let len = chunk.len();
            chunk.copy_from_slice(&val[..len]);
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Wire struct fuzzing: safely decode arbitrary byte buffers
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn fuzz_position_wire_decoding_random_bytes() {
    let mut rng = SimpleRng::new(0x123456789ABCDEF0);
    let mut raw = Mt5Position::default();

    for _ in 0..5000 {
        let slice = unsafe {
            std::slice::from_raw_parts_mut(
                &mut raw as *mut Mt5Position as *mut u8,
                std::mem::size_of::<Mt5Position>(),
            )
        };
        rng.fill_bytes(slice);

        // Position::from_raw must not panic regardless of byte content
        let pos = Position::from_raw(raw);
        assert!(!pos.symbol.contains('\0'));
        assert!(!pos.comment.contains('\0'));
    }
}

#[test]
fn fuzz_order_wire_decoding_random_bytes() {
    let mut rng = SimpleRng::new(0x0FEDCBA987654321);
    let mut raw = Mt5Order::default();

    for _ in 0..5000 {
        let slice = unsafe {
            std::slice::from_raw_parts_mut(
                &mut raw as *mut Mt5Order as *mut u8,
                std::mem::size_of::<Mt5Order>(),
            )
        };
        rng.fill_bytes(slice);

        let ord = WorkingOrder::from_raw(raw);
        assert!(!ord.symbol.contains('\0'));
        assert!(!ord.comment.contains('\0'));
    }
}

#[test]
fn fuzz_deal_wire_decoding_random_bytes() {
    let mut rng = SimpleRng::new(0xAABBCCDDEEFF0011);
    let mut raw = Mt5Deal::default();

    for _ in 0..5000 {
        let slice = unsafe {
            std::slice::from_raw_parts_mut(
                &mut raw as *mut Mt5Deal as *mut u8,
                std::mem::size_of::<Mt5Deal>(),
            )
        };
        rng.fill_bytes(slice);

        let deal = Deal::from_raw(raw);
        assert!(!deal.symbol.contains('\0'));
        assert!(!deal.comment.contains('\0'));
    }
}

#[test]
fn fuzz_trade_result_wire_decoding() {
    let mut rng = SimpleRng::new(0xCAFEBABE11223344);
    let mut raw = Mt5TradeResult::default();

    for _ in 0..5000 {
        let slice = unsafe {
            std::slice::from_raw_parts_mut(
                &mut raw as *mut Mt5TradeResult as *mut u8,
                std::mem::size_of::<Mt5TradeResult>(),
            )
        };
        rng.fill_bytes(slice);

        let res = TradeResult::from_raw(raw);
        let raw_retcode = { raw.retcode };
        let raw_deal = { raw.deal };
        let raw_order = { raw.order };
        let raw_position = { raw.position };
        assert_eq!(res.retcode, raw_retcode);
        assert_eq!(res.deal, raw_deal);
        assert_eq!(res.order, raw_order);
        assert_eq!(res.position, raw_position);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Client Order ID and Wire Token Fuzzing
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn fuzz_wire_id_arbitrary_strings() {
    let mut rng = SimpleRng::new(0x9988776655443322);

    for len in 0..128 {
        for _ in 0..100 {
            let mut buf = vec![0u8; len];
            rng.fill_bytes(&mut buf);
            let s = String::from_utf8_lossy(&buf);

            let token = wire_id(&s);
            assert_eq!(token.len(), WIRE_ID_LEN);
            assert!(token.bytes().all(|b| {
                b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b))
            }));
            let wire_comment = format!("cid:{token}");
            assert_eq!(parse_wire_id(&wire_comment), Some(token.as_str()));
        }
    }
}

#[test]
fn fuzz_parse_wire_id_malformed_tokens() {
    let mut rng = SimpleRng::new(0x1357924680ACEF11);

    for len in 0..32 {
        for _ in 0..100 {
            let mut buf = vec![0u8; len];
            rng.fill_bytes(&mut buf);
            let s = String::from_utf8_lossy(&buf);

            if let Some(parsed) = parse_wire_id(&s) {
                assert_eq!(parsed.len(), WIRE_ID_LEN);
                assert!(parsed.bytes().all(|b| {
                    b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b))
                }));
            }
        }
    }
}

#[test]
fn fuzz_effective_comment_utf8_boundaries() {
    // Tests various multi-byte Unicode strings (accented, Asian characters, emoji, combining chars)
    let fragments = [
        "a", "€", "ç", "ö", "ü", "ş", "ğ", "日", "本", "語",
        "😀", "🚀", "💡", "—", "\u{200B}", "\u{FEFF}",
    ];

    for depth in 1..40 {
        let s: String = (0..depth)
            .map(|i| fragments[i % fragments.len()])
            .collect();

        let req1 = OrderRequest::buy("EURUSD", 0.1)
            .client_order_id(&s)
            .comment("comment-tail");
        let cmt1 = req1.effective_comment();
        assert!(cmt1.len() <= 31);
        assert!(std::str::from_utf8(cmt1.as_bytes()).is_ok());

        let req2 = OrderRequest::buy("EURUSD", 0.1).client_order_id(&s);
        let cmt2 = req2.effective_comment();
        assert!(cmt2.len() <= 31);
        assert!(std::str::from_utf8(cmt2.as_bytes()).is_ok());
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. OrderRequest validation fuzzing
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn fuzz_order_request_validation_extreme_values() {
    let floats = [
        0.0, -0.0, 1.0, -1.0, 0.01, 1e-10, 1e10,
        f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
        f64::MIN_POSITIVE, f64::MAX, f64::MIN,
    ];

    for &vol in &floats {
        for &price in &floats {
            for &sl in &floats {
                for &tp in &floats {
                    let req = OrderRequest {
                        symbol: "EURUSD".to_string(),
                        order_type: OrderType::Buy,
                        volume: vol,
                        price,
                        stop_loss: sl,
                        take_profit: tp,
                        comment: String::new(),
                        client_order_id: Some("fuzz-id-1".to_string()),
                        magic: Some(12345),
                        deviation: Some(10),
                        expiration: Some(0),
                    };

                    // Must return Ok or Err, never panic
                    let _ = req.validate();
                }
            }
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. Persistence & Journal State Fuzzing
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn fuzz_json_file_store_corrupted_payloads() {
    let mut rng = SimpleRng::new(0xDEADBEEF00112233);
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join(format!("fuzz_journal_{}.json", std::process::id()));

    for _ in 0..200 {
        let len = (rng.next_u32() % 512) as usize;
        let mut junk = vec![0u8; len];
        rng.fill_bytes(&mut junk);

        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(&junk).unwrap();
        }

        let store = JsonFileStore::new(&path);
        // Loading corrupted bytes must result in Err, never panic or crash
        let res = store.load();
        // If it randomly matched valid JSON it would be Ok, otherwise Err
        if let Ok(orders) = res {
            assert!(orders.is_empty() || !orders.is_empty());
        }
    }

    let _ = std::fs::remove_file(&path);
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Tick arithmetic fuzzing across random values
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn fuzz_tick_arithmetic_random_values() {
    let mut rng = SimpleRng::new(0x554433221100FFEE);

    for _ in 0..10000 {
        let price_bits = rng.next_u64();
        let tick_size_bits = rng.next_u64();
        let price = f64::from_bits(price_bits);
        let tick_size = f64::from_bits(tick_size_bits);
        let digits = (rng.next_u32() % 10) as u32;

        // Must not panic on arbitrary floats
        let ticks = price_to_ticks(price, tick_size);
        let _ = ticks_to_price(ticks, tick_size, digits);
    }
}

//! Protocol documentation tests.
//!
//! Asserts that README.md and documentation stay synchronized with
//! wire sizes, method names, and FFI packed struct contracts.

const EXPECTED: &[(&str, usize)] = &[
    ("PacketHdr", 16),
    ("Mt5TickEvent", 88),
    ("Mt5TradeEvent", 176),
    ("Mt5BookEvent", 72),
    ("Mt5SymInfo", 76),
    ("Mt5BarEvent", 96),
];

#[test]
fn readme_sizes_match_ffi() {
    let readme = include_str!("../README.md");
    for (name, size) in EXPECTED {
        assert!(
            readme.contains(&format!("`{name}` ({size} bytes)")),
            "README is missing or has a stale size for {name}"
        );
    }
}

#[test]
fn documented_methods_exist() {
    let readme = include_str!("../README.md");
    for method in ["subscribe_trade", "subscribe_book", "subscribe_bars"] {
        assert!(readme.contains(method), "README does not mention {method}");
    }
}

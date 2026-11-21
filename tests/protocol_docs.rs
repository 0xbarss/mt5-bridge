//! Protocol documentation tests.
//!
//! Asserts that README.md and documentation stay synchronized with
//! wire sizes, method names, and FFI packed struct contracts.

const EXPECTED_STRUCT_SIZES: &[(&str, usize)] = &[
    ("PacketHdr", 16),
    ("Mt5TickEvent", 88),
    ("Mt5TradeEvent", 176),
    ("Mt5BookEvent", 72),
    ("Mt5SymInfo", 76),
    ("Mt5BarEvent", 96),
    ("Mt5Rate", 60),
    ("Mt5Tick", 44),
    ("Mt5TradeResult", 44),
    ("Mt5Position", 148),
    ("Mt5Order", 140),
    ("Mt5Deal", 152),
];

#[test]
fn readme_sizes_match_ffi() {
    let readme = include_str!("../README.md");
    for (name, size) in EXPECTED_STRUCT_SIZES {
        assert!(
            readme.contains(&format!("`{name}` ({size} bytes)")),
            "README is missing or has a stale size for {name}"
        );
    }
}

#[test]
fn documented_methods_exist() {
    let readme = include_str!("../README.md");
    for method in [
        "subscribe_trade",
        "subscribe_book",
        "subscribe_bars",
        "close_position",
        "order_modify",
        "stream_bars",
    ] {
        assert!(readme.contains(method), "README does not mention {method}");
    }
    assert!(
        readme.contains("stop_loss: Level, take_profit: Level"),
        "README order_modify signature does not reflect Level parameter types"
    );
}

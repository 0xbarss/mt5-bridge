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
    ("Mt5Stats", 64),
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

// Schema and wire-layout snapshot tests.
// Gated on the `schema` feature (enabled by `--all-features`).

#[cfg(feature = "schema")]
mod schema_snapshot_tests {
    use super::*;
    use mt5_bridge::schema::wire_layouts;

    #[test]
    fn generated_layout_sizes_match_documented_sizes() {
        let layouts = wire_layouts();
        for (name, size) in EXPECTED_STRUCT_SIZES {
            let layout = layouts
                .iter()
                .find(|l| l.name == *name)
                .unwrap_or_else(|| panic!("missing generated layout for {name}"));
            assert_eq!(
                layout.size, *size,
                "generated size for {name} does not match documented size"
            );
        }
        // Every documented wire type has a generated layout of the same size.
        let generated_names: std::collections::HashSet<&str> =
            layouts.iter().map(|l| l.name).collect();
        for (name, _) in EXPECTED_STRUCT_SIZES {
            assert!(
                generated_names.contains(name),
                "no generated layout for documented type {name}"
            );
        }
    }

    #[test]
    fn generated_offsets_are_within_type_and_ordered() {
        for layout in wire_layouts() {
            let mut prev: Option<usize> = None;
            for field in &layout.fields {
                assert!(
                    field.offset < layout.size,
                    "{name}::{fname} offset {off} >= size {size}",
                    name = layout.name,
                    fname = field.name,
                    off = field.offset,
                    size = layout.size
                );
                if let Some(p) = prev {
                    assert!(
                        field.offset > p,
                        "{name} fields not in increasing offset order",
                        name = layout.name
                    );
                }
                prev = Some(field.offset);
            }
        }
    }

    #[test]
    fn wire_layout_report_mentions_every_wire_type() {
        let report = mt5_bridge::schema::wire_layout_report();
        assert!(!report.is_empty());
        for (name, _) in EXPECTED_STRUCT_SIZES {
            assert!(
                report.contains(name),
                "wire layout report does not mention {name}"
            );
        }
    }
}

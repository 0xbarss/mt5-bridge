//! Cross-language drift checks.
//!
//! The wire protocol is implemented three times: Rust (`src/ffi.rs`), the C++ DLL
//! (`bridge_dll/`) and the MQL5 EA (`mql5/Experts/mt5_bridge.mq5`). The EA can only be compiled
//! inside MetaEditor, so these tests parse the other two sources as text and fail loudly if
//! the constants that must agree ever diverge — protocol version, command IDs, packed struct
//! sizes, and the client-order-id wire token format.

use mt5_bridge::ffi::*;
use mt5_bridge::{wire_id, WIRE_ID_LEN};
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(format!("{}/{rel}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

/// Value of `#define NAME <int>` (first token after the name; trailing comments ignored).
fn define(src: &str, name: &str) -> u64 {
    for line in src.lines() {
        let mut it = line.split_whitespace();
        if it.next() == Some("#define") && it.next() == Some(name) {
            let v = it.next().unwrap_or_else(|| panic!("#define {name} has no value"));
            return v.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().unwrap();
        }
    }
    panic!("#define {name} not found");
}

/// `NAME = <int>,` inside the C++ `enum Cmd`.
fn cpp_enum(src: &str, name: &str) -> u64 {
    for line in src.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix(name) {
            if let Some(v) = rest.trim_start().strip_prefix('=') {
                return v.trim().trim_end_matches(',').split_whitespace().next().unwrap().parse().unwrap();
            }
        }
    }
    panic!("{name} not found");
}

#[test]
fn protocol_version_agrees_across_rust_dll_and_ea() {
    let h = read("bridge_dll/mt5_bridge.h");
    let ea = read("mql5/Experts/mt5_bridge.mq5");
    assert_eq!(define(&h, "MT5_BRIDGE_PROTOCOL_VERSION"), PROTOCOL_VERSION as u64, "DLL header");
    assert_eq!(define(&ea, "PROTOCOL_VERSION"), PROTOCOL_VERSION as u64, "EA");
}

#[test]
fn command_ids_agree_across_rust_dll_and_ea() {
    let cpp = read("bridge_dll/mt5_bridge.cpp");
    let ea = read("mql5/Experts/mt5_bridge.mq5");
    let table = [
        ("CMD_INIT", ProtocolCmd::Init),
        ("CMD_SHUTDOWN", ProtocolCmd::Shutdown),
        ("CMD_RATES", ProtocolCmd::Rates),
        ("CMD_ACCOUNT", ProtocolCmd::Account),
        ("CMD_ORDER_SEND", ProtocolCmd::OrderSend),
        ("CMD_ORDER_CLOSE", ProtocolCmd::OrderClose),
        ("CMD_ORDER_MODIFY", ProtocolCmd::OrderModify),
        ("CMD_SYM_TICK", ProtocolCmd::SymTick),
        ("CMD_SYM_INFO", ProtocolCmd::SymInfo),
        ("CMD_POSITIONS_GET", ProtocolCmd::PositionsGet),
        ("CMD_ORDERS_GET", ProtocolCmd::OrdersGet),
        ("CMD_DEALS_GET", ProtocolCmd::DealsGet),
    ];
    for (name, cmd) in table {
        assert_eq!(cpp_enum(&cpp, name), cmd as u64, "DLL {name}");
        assert_eq!(define(&ea, name), cmd as u64, "EA {name}");
    }
    // …and the EA actually dispatches the new command.
    assert!(ea.contains("case CMD_DEALS_GET:") && ea.contains("void HandleDealsGet("));
    assert!(cpp.contains("int DealsGet(Mt5Deal* buf"));
}

#[test]
fn packed_struct_sizes_agree_between_rust_and_dll_static_asserts() {
    let h = read("bridge_dll/mt5_bridge.h");
    let expect = [
        ("Mt5SymInfo", std::mem::size_of::<Mt5SymInfo>()),
        ("Mt5Rate", std::mem::size_of::<Mt5Rate>()),
        ("Mt5Tick", std::mem::size_of::<Mt5Tick>()),
        ("Mt5TradeResult", std::mem::size_of::<Mt5TradeResult>()),
        ("Mt5Position", std::mem::size_of::<Mt5Position>()),
        ("Mt5Order", std::mem::size_of::<Mt5Order>()),
        ("Mt5Deal", std::mem::size_of::<Mt5Deal>()),
    ];
    for (name, rust_size) in expect {
        let needle = format!("static_assert(sizeof({name}) == ");
        let line = h.lines().find(|l| l.contains(&needle)).unwrap_or_else(|| panic!("no static_assert for {name}"));
        let n: usize = line.split("== ").nth(1).unwrap().split(',').next().unwrap().trim().parse().unwrap();
        assert_eq!(n, rust_size, "{name}: DLL says {n}, Rust says {rust_size}");
    }
}

#[test]
fn deal_struct_field_order_is_identical_in_the_dll_header() {
    // Sizes alone would miss two same-size fields being swapped, so also compare field order.
    let h = read("bridge_dll/mt5_bridge.h");
    let start = h.find("typedef struct {\n    uint64_t ticket;\n    uint64_t order;").expect("Mt5Deal typedef");
    let end = h[start..].find("} Mt5Deal;").unwrap() + start;
    let fields: Vec<String> = h[start..end]
        .lines()
        .skip(1)
        .filter_map(|l| {
            let l = l.split("/*").next().unwrap().trim().trim_end_matches(';');
            l.split_whitespace().last().map(|s| s.split('[').next().unwrap().to_string())
        })
        .collect();
    assert_eq!(
        fields,
        ["ticket", "order", "position_id", "time", "type", "entry", "magic", "volume", "price",
         "commission", "swap", "profit", "symbol", "comment"]
    );
    // The Rust wire struct must use the same order (checked via a byte-level round trip).
    let d = Mt5Deal { ticket: 1, order: 2, position_id: 3, time: 4, deal_type: 5, entry: 6, magic: 7,
        volume: 8.0, price: 9.0, commission: 10.0, swap: 11.0, profit: 12.0, ..Default::default() };
    let bytes: [u8; 152] = unsafe { std::mem::transmute(d) };
    let u64_at = |o: usize| u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
    let i32_at = |o: usize| i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let f64_at = |o: usize| f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
    assert_eq!((u64_at(0), u64_at(8), u64_at(16), u64_at(24)), (1, 2, 3, 4));
    assert_eq!((i32_at(32), i32_at(36), u64_at(40)), (5, 6, 7));
    assert_eq!((f64_at(48), f64_at(56), f64_at(64), f64_at(72), f64_at(80)), (8.0, 9.0, 10.0, 11.0, 12.0));
}

#[test]
fn ea_token_format_matches_the_rust_wire_id() {
    let ea = read("mql5/Experts/mt5_bridge.mq5");
    assert_eq!(define(&ea, "WIRE_ID_LEN"), WIRE_ID_LEN as u64);

    // The EA's IsWireTokenChar must accept exactly the Crockford alphabet the Rust side emits:
    // 0-9 and A-Z minus I, L, O, U.
    let body = &ea[ea.find("bool IsWireTokenChar").expect("IsWireTokenChar")..];
    let body = &body[..body.find("\n}\n").unwrap()];
    for excluded in ["'I'", "'L'", "'O'", "'U'"] {
        assert!(body.contains(&format!("c != {excluded}")), "EA must exclude {excluded}");
    }
    let sample = wire_id("anything at all");
    assert_eq!(sample.len(), WIRE_ID_LEN);
    assert!(sample.bytes().all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b))));

    // The EA must not fall back to treating arbitrary comments as keys (the old bug).
    let extract = &ea[ea.find("string ExtractClientOrderId").unwrap()..];
    let extract = &extract[..extract.find("\n}\n").unwrap()];
    assert!(!extract.contains("return cmt;"), "ExtractClientOrderId must not return the raw comment");
    assert!(extract.contains("return \"\";"));
}

#[test]
fn dll_deals_request_layout_matches_the_ea_handler() {
    // Request order is from, to, magic, symbol, capacity on both sides.
    let cpp = read("bridge_dll/mt5_bridge.cpp");
    let ea = read("mql5/Experts/mt5_bridge.mq5");
    let dll = &cpp[cpp.find("int DealsGet(").unwrap()..];
    let dll = &dll[..dll.find("\n}\n").unwrap()];
    let order: Vec<usize> = ["p.i64(from)", "p.i64(to)", "p.u64(magic_filter)", "p.str(", "p.i32(buf_capacity)"]
        .iter().map(|k| dll.find(k).unwrap_or_else(|| panic!("DLL missing {k}"))).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "DLL packs in the wrong order");

    let h = &ea[ea.find("void HandleDealsGet(").unwrap()..];
    let h = &h[..h.find("\n}\n").unwrap()];
    let order: Vec<usize> = ["SafeUnpackI64(payload, off, len, from_utc)", "SafeUnpackI64(payload, off, len, to_utc)",
        "SafeUnpackU64(payload, off, len, magic_filter)", "SafeUnpackStr(payload, off, len, symbol_filter)",
        "SafeUnpackI32(payload, off, len, max_items)"]
        .iter().map(|k| h.find(k).unwrap_or_else(|| panic!("EA missing {k}"))).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "EA unpacks in the wrong order");

    // EA response field order == Mt5Deal field order.
    let packs: Vec<usize> = ["PackU64(data, ticket)", "DEAL_ORDER", "DEAL_POSITION_ID", "PackI64(data, deal_time)",
        "PackI32(data, (deal_type", "DEAL_ENTRY", "PackU64(data, deal_magic)", "DEAL_VOLUME", "DEAL_PRICE",
        "DEAL_COMMISSION", "DEAL_SWAP", "DEAL_PROFIT", "PackFixedString32(data, sym)", "DEAL_COMMENT"]
        .iter().map(|k| h.rfind(k).unwrap_or_else(|| panic!("EA missing {k}"))).collect();
    assert!(packs.windows(2).all(|w| w[0] < w[1]), "EA packs Mt5Deal fields in the wrong order: {packs:?}");
}

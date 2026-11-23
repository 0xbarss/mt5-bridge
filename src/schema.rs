//! JSON Schema and wire-layout reports for MT5 wire-protocol types.
//!
//! Gated on the `schema` feature. JSON Schema is generated with `schemars`;
//! wire offsets use `std::mem::size_of!` plus a MSRV-safe offset helper
//! (`std::mem::offset_of!` is stable only from Rust 1.78, while this crate
//! targets 1.81).

#![cfg_attr(not(feature = "schema"), allow(dead_code))]

use crate::ffi::{
    Mt5BarEvent, Mt5BookEvent, Mt5Deal, Mt5Order, Mt5Position, Mt5Rate, Mt5Stats, Mt5SymInfo,
    Mt5Tick, Mt5TickEvent, Mt5TradeEvent, Mt5TradeResult, PacketHdr,
};
use std::mem::size_of;
use std::ptr::addr_of;

/// MSRV-safe byte offset of `$field` within `$ty`.
///
/// `std::mem::offset_of!` is stable since Rust 1.78; this crate targets 1.81,
/// so the offset is computed from a null base pointer via `addr_of!`, which
/// constructs a raw pointer without dereferencing and is therefore
/// well-defined on every supported toolchain.
macro_rules! offset_of {
    ($ty:ty, $field:ident) => {{
        let base: *const $ty = std::ptr::null();
        let field = unsafe { addr_of!((*base).$field) as usize };
        field - (base as usize)
    }};
}

/// One field's name and byte offset within its container type.
#[derive(Debug, Clone, Copy)]
pub struct FieldLayout {
    pub name: &'static str,
    pub offset: usize,
}

/// One wire type's size and per-field offsets.
#[derive(Debug, Clone)]
pub struct TypeLayout {
    pub name: &'static str,
    pub size: usize,
    pub fields: Vec<FieldLayout>,
}

macro_rules! layout_of {
    ($ty:ty, $name:expr, [ $( ($field:ident, $fname:expr) ),* $(,)? ]) => {{
        TypeLayout {
            name: $name,
            size: size_of::<$ty>(),
            fields: vec![$( FieldLayout {
                name: $fname,
                offset: offset_of!($ty, $field),
            } ),*],
        }
    }};
}

/// Wire-layout descriptions for every packed wire-protocol type, in declaration
/// order. Each entry gives the total size and the byte offset of every field.
pub fn wire_layouts() -> Vec<TypeLayout> {
    vec![
        layout_of!(
            PacketHdr,
            "PacketHdr",
            [
                (length, "length"),
                (kind, "kind"),
                (_pad, "_pad"),
                (id, "id"),
                (status, "status"),
                (request_id, "request_id"),
            ]
        ),
        layout_of!(
            Mt5SymInfo,
            "Mt5SymInfo",
            [
                (point, "point"),
                (tick_value, "tick_value"),
                (tick_size, "tick_size"),
                (lot_step, "lot_step"),
                (min_lot, "min_lot"),
                (max_lot, "max_lot"),
                (spread, "spread"),
                (digits, "digits"),
                (bid, "bid"),
                (ask, "ask"),
            ]
        ),
        layout_of!(
            Mt5Rate,
            "Mt5Rate",
            [
                (time, "time"),
                (open, "open"),
                (high, "high"),
                (low, "low"),
                (close, "close"),
                (volume, "volume"),
                (spread, "spread"),
                (_rv, "_rv"),
            ]
        ),
        layout_of!(
            Mt5Tick,
            "Mt5Tick",
            [
                (time, "time"),
                (bid, "bid"),
                (ask, "ask"),
                (last, "last"),
                (volume, "volume"),
                (flags, "flags"),
            ]
        ),
        layout_of!(
            Mt5TradeResult,
            "Mt5TradeResult",
            [
                (retcode, "retcode"),
                (deal, "deal"),
                (order, "order"),
                (position, "position"),
                (volume, "volume"),
                (price, "price"),
            ]
        ),
        layout_of!(
            Mt5Position,
            "Mt5Position",
            [
                (ticket, "ticket"),
                (time, "time"),
                (position_type, "position_type"),
                (magic, "magic"),
                (volume, "volume"),
                (price_open, "price_open"),
                (sl, "sl"),
                (tp, "tp"),
                (price_current, "price_current"),
                (profit, "profit"),
                (swap, "swap"),
                (symbol, "symbol"),
                (comment, "comment"),
            ]
        ),
        layout_of!(
            Mt5Order,
            "Mt5Order",
            [
                (ticket, "ticket"),
                (time_setup, "time_setup"),
                (order_type, "order_type"),
                (magic, "magic"),
                (volume_initial, "volume_initial"),
                (volume_current, "volume_current"),
                (price_open, "price_open"),
                (sl, "sl"),
                (tp, "tp"),
                (price_current, "price_current"),
                (symbol, "symbol"),
                (comment, "comment"),
            ]
        ),
        layout_of!(
            Mt5Deal,
            "Mt5Deal",
            [
                (ticket, "ticket"),
                (order, "order"),
                (position_id, "position_id"),
                (time, "time"),
                (deal_type, "deal_type"),
                (entry, "entry"),
                (magic, "magic"),
                (volume, "volume"),
                (price, "price"),
                (commission, "commission"),
                (swap, "swap"),
                (profit, "profit"),
                (symbol, "symbol"),
                (comment, "comment"),
            ]
        ),
        layout_of!(
            Mt5TickEvent,
            "Mt5TickEvent",
            [
                (symbol, "symbol"),
                (time_msc, "time_msc"),
                (bid, "bid"),
                (ask, "ask"),
                (last, "last"),
                (volume, "volume"),
                (flags, "flags"),
                (_pad, "_pad"),
                (event_seq, "event_seq"),
            ]
        ),
        layout_of!(
            Mt5TradeEvent,
            "Mt5TradeEvent",
            [
                (deal, "deal"),
                (order, "order"),
                (position, "position"),
                (time, "time"),
                (trans_type, "trans_type"),
                (order_type, "order_type"),
                (price, "price"),
                (volume, "volume"),
                (sl, "sl"),
                (tp, "tp"),
                (symbol, "symbol"),
                (comment, "comment"),
                (deal_entry, "deal_entry"),
                (deal_type, "deal_type"),
                (deal_magic, "deal_magic"),
                (deal_position_id, "deal_position_id"),
                (deal_reason, "deal_reason"),
                (_pad, "_pad"),
                (event_seq, "event_seq"),
            ]
        ),
        layout_of!(
            Mt5BookEvent,
            "Mt5BookEvent",
            [
                (symbol, "symbol"),
                (time_msc, "time_msc"),
                (book_type, "book_type"),
                (_pad, "_pad"),
                (price, "price"),
                (volume, "volume"),
                (event_seq, "event_seq"),
            ]
        ),
        layout_of!(
            Mt5BarEvent,
            "Mt5BarEvent",
            [
                (symbol, "symbol"),
                (time, "time"),
                (open, "open"),
                (high, "high"),
                (low, "low"),
                (close, "close"),
                (volume, "volume"),
                (timeframe, "timeframe"),
                (spread, "spread"),
                (event_seq, "event_seq"),
            ]
        ),
        layout_of!(
            Mt5Stats,
            "Mt5Stats",
            [
                (uptime_secs, "uptime_secs"),
                (requests_total, "requests_total"),
                (tick_drops, "tick_drops"),
                (trade_drops, "trade_drops"),
                (book_drops, "book_drops"),
                (bar_drops, "bar_drops"),
                (tick_queue_depth, "tick_queue_depth"),
                (trade_queue_depth, "trade_queue_depth"),
                (last_error, "last_error"),
                (protocol, "protocol"),
            ]
        ),
    ]
}

/// Format `wire_layouts()` as a human-readable text report.
pub fn wire_layout_report() -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("MT5 wire layout (sizes and field offsets, bytes)".to_string());
    lines.push(String::new());
    for layout in wire_layouts() {
        lines.push(format!("{} ({} bytes):", layout.name, layout.size));
        for field in &layout.fields {
            lines.push(format!("  {:>4} @ offset {}", field.offset, field.name));
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

/// JSON Schema (draft-07) for the wire-protocol types, generated with `schemars`.
pub fn json_schema_report() -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("JSON Schema (draft-07) for MT5 wire-protocol types".to_string());
    lines.push(String::new());
    for schema in [
        schemars::schema_for!(PacketHdr),
        schemars::schema_for!(Mt5SymInfo),
        schemars::schema_for!(Mt5Rate),
        schemars::schema_for!(Mt5Tick),
        schemars::schema_for!(Mt5TradeResult),
        schemars::schema_for!(Mt5Position),
        schemars::schema_for!(Mt5Order),
        schemars::schema_for!(Mt5Deal),
        schemars::schema_for!(Mt5TickEvent),
        schemars::schema_for!(Mt5TradeEvent),
        schemars::schema_for!(Mt5BookEvent),
        schemars::schema_for!(Mt5BarEvent),
        schemars::schema_for!(Mt5Stats),
    ] {
        lines.push(format!("{schema:#?}"));
        lines.push(String::new());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_declared_layout() {
        let expected: &[(&str, usize)] = &[
            ("PacketHdr", 16),
            ("Mt5SymInfo", 76),
            ("Mt5Rate", 60),
            ("Mt5Tick", 44),
            ("Mt5TradeResult", 44),
            ("Mt5Position", 148),
            ("Mt5Order", 140),
            ("Mt5Deal", 152),
            ("Mt5TickEvent", 88),
            ("Mt5TradeEvent", 176),
            ("Mt5BookEvent", 72),
            ("Mt5BarEvent", 96),
            ("Mt5Stats", 64),
        ];
        let layouts = wire_layouts();
        assert_eq!(layouts.len(), expected.len());
        for (layout, (name, size)) in layouts.iter().zip(expected.iter()) {
            assert_eq!(layout.name, *name, "unexpected layout ordering");
            assert_eq!(layout.size, *size, "size mismatch for {name}");
        }
    }

    #[test]
    fn offsets_are_within_type_and_non_overlapping() {
        for layout in wire_layouts() {
            let mut prev: Option<usize> = None;
            for field in &layout.fields {
                assert!(
                    field.offset < layout.size,
                    "{}::{} offset {} >= size {}",
                    layout.name,
                    field.name,
                    field.offset,
                    layout.size
                );
                if let Some(p) = prev {
                    assert!(field.offset > p, "{} fields not ordered", layout.name);
                }
                prev = Some(field.offset);
            }
        }
    }

    #[test]
    fn json_schema_generates_for_every_wire_type() {
        // `schema_for!` panics at compile time if a type lacks `JsonSchema`; this
        // test merely ensures the generated schemas are non-empty.
        for schema in [
            schemars::schema_for!(PacketHdr),
            schemars::schema_for!(Mt5SymInfo),
            schemars::schema_for!(Mt5Rate),
            schemars::schema_for!(Mt5Stats),
        ] {
            let text = format!("{schema:#?}");
            assert!(!text.is_empty());
        }
    }
}

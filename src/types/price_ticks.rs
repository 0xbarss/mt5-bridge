use crate::constants::PRICE_TOLERANCE;

/// Converts a floating-point price into an integer number of price ticks based on tick size.
pub fn price_to_ticks(price: f64, tick_size: f64) -> i64 {
    if tick_size <= 0.0 || !price.is_finite() {
        0
    } else {
        ((price / tick_size) + PRICE_TOLERANCE).round() as i64
    }
}

/// Converts an integer tick count into a normalized floating-point price according to tick size and digits.
pub fn ticks_to_price(ticks: i64, tick_size: f64, digits: u32) -> f64 {
    let raw = (ticks as f64) * tick_size;
    let factor = 10f64.powi(digits as i32);
    (raw * factor).round() / factor
}

/// Calculate a Stop Loss price based on tick distance from an entry price, avoiding floating-point drift.
pub fn calculate_sl_ticks(
    entry: f64,
    distance_ticks: i64,
    is_buy: bool,
    tick_size: f64,
    digits: u32,
) -> f64 {
    let entry_ticks = price_to_ticks(entry, tick_size);
    // Saturating arithmetic: `i64::MIN.abs()` overflows (panic in debug, silent wrap to a
    // *negative* distance in release, which would put a buy's stop above entry).
    let dist = distance_ticks.saturating_abs();
    let sl_ticks = if is_buy {
        entry_ticks.saturating_sub(dist)
    } else {
        entry_ticks.saturating_add(dist)
    };
    ticks_to_price(sl_ticks, tick_size, digits)
}

/// Calculate a Take Profit price based on tick distance from an entry price, avoiding floating-point drift.
pub fn calculate_tp_ticks(
    entry: f64,
    distance_ticks: i64,
    is_buy: bool,
    tick_size: f64,
    digits: u32,
) -> f64 {
    let entry_ticks = price_to_ticks(entry, tick_size);
    let dist = distance_ticks.saturating_abs();
    let tp_ticks = if is_buy {
        entry_ticks.saturating_add(dist)
    } else {
        entry_ticks.saturating_sub(dist)
    };
    ticks_to_price(tp_ticks, tick_size, digits)
}

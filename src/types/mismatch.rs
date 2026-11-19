use crate::constants::PRICE_TOLERANCE;
use serde::{Deserialize, Serialize};

/// Category of disagreement between what was requested and what the broker reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MismatchKind {
    Symbol,
    /// Buy vs. sell.
    Direction,
    /// Order type differs (e.g. requested `BuyLimit`, found `BuyStop`).
    OrderType,
    Volume,
    Price,
    StopLoss,
    TakeProfit,
    Magic,
}

/// One attribute of a broker-side position/order that differs from the original request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeMismatch {
    pub kind: MismatchKind,
    /// What the local request asked for.
    pub expected: String,
    /// What the broker reports.
    pub actual: String,
}

pub fn approx_volume_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-8
}

pub fn approx_price_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= PRICE_TOLERANCE * a.abs().max(b.abs()).max(1.0)
}

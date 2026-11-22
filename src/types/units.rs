//! Typed domain newtypes to prevent unit mistakes (points vs price, lots vs volume,
//! tickets vs magic).
//!
//! Each newtype wraps a primitive and carries its unit in the type, so values of
//! different units cannot be mixed accidentally. The existing primitive API is kept;
//! typed accessors are added alongside.
//!
//! Conversions are infallible where lossless (`From`) and fallible where the target
//! could reject the value (`TryFrom`, e.g. negative volume).

use crate::types::price_ticks::price_to_ticks;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A price quote in the symbol's native units.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Price(pub f64);

/// A trade volume (lots) in the symbol's native units.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Volume(pub f64);

/// A unique order ticket number assigned by the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Ticket(pub u64);

/// An EA/order magic number identifying ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Magic(pub u64);

/// A trading symbol (e.g. "EURUSD").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol(pub String);

/// A price distance in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Points(pub i64);

// ---- Price ----

impl Price {
    /// Round to the nearest multiple of `tick_size`.
    pub fn round_to(self, tick_size: f64) -> Self {
        let ticks = price_to_ticks(self.0, tick_size);
        Self(ticks as f64 * tick_size)
    }

    /// Number of `tick_size` steps represented by this price.
    pub fn ticks(self, tick_size: f64) -> i64 {
        price_to_ticks(self.0, tick_size)
    }

    pub fn into_inner(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Price {
    type Error = &'static str;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value.is_nan() || value.is_infinite() {
            return Err("price must be finite");
        }
        Ok(Self(value))
    }
}

impl From<Price> for f64 {
    fn from(value: Price) -> f64 {
        value.0
    }
}

impl std::ops::Add for Price {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Price {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---- Volume ----

impl Volume {
    /// Create a volume from a lot size.
    pub fn lots(lots: f64) -> Self {
        Self(lots)
    }

    pub fn into_inner(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Volume {
    type Error = &'static str;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value <= 0.0 || value.is_nan() || value.is_infinite() {
            return Err("volume must be positive and finite");
        }
        Ok(Self(value))
    }
}

impl From<Volume> for f64 {
    fn from(value: Volume) -> f64 {
        value.0
    }
}

impl std::ops::Mul<Price> for Volume {
    type Output = f64;
    fn mul(self, rhs: Price) -> f64 {
        self.0 * rhs.0
    }
}

impl fmt::Display for Volume {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---- Ticket ----

impl Ticket {
    pub fn into_inner(self) -> u64 {
        self.0
    }
}

impl From<u64> for Ticket {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Ticket> for u64 {
    fn from(value: Ticket) -> u64 {
        value.0
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---- Magic ----

impl Magic {
    pub fn into_inner(self) -> u64 {
        self.0
    }
}

impl From<u64> for Magic {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Magic> for u64 {
    fn from(value: Magic) -> u64 {
        value.0
    }
}

impl PartialEq<u64> for Magic {
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}

impl fmt::Display for Magic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---- Symbol ----

impl Symbol {
    pub fn into_inner(self) -> String {
        self.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Symbol {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<Symbol> for String {
    fn from(value: Symbol) -> String {
        value.0
    }
}

impl From<&str> for Symbol {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---- Points ----

impl Points {
    pub fn into_inner(self) -> i64 {
        self.0
    }
}

impl From<i64> for Points {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<Points> for i64 {
    fn from(value: Points) -> i64 {
        value.0
    }
}

impl std::ops::Add for Points {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Points {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl fmt::Display for Points {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_round_to_and_ticks() {
        let p = Price::try_from(1.100137).unwrap();
        let r = p.round_to(0.00001);
        assert!((r.into_inner() - 1.10014).abs() < 1e-9);
        assert_eq!(p.ticks(0.00001), 110014);
    }

    #[test]
    fn price_rejects_non_finite() {
        assert!(Price::try_from(f64::NAN).is_err());
        assert!(Price::try_from(f64::INFINITY).is_err());
        assert!(Price::try_from(1.1).is_ok());
    }

    #[test]
    fn volume_rejects_non_positive() {
        assert!(Volume::try_from(0.0).is_err());
        assert!(Volume::try_from(-1.0).is_err());
        assert!(Volume::try_from(f64::NAN).is_err());
        assert!(Volume::try_from(0.5).is_ok());
    }

    #[test]
    fn volume_times_price() {
        let v: Volume = Volume::try_from(0.5).unwrap();
        let p: Price = Price::try_from(1.10).unwrap();
        assert!((v * p - 0.55).abs() < 1e-9);
    }

    #[test]
    fn magic_eq_u64() {
        let m: Magic = 20240101.into();
        assert!(m == 20240101u64);
        assert!(m != 424242u64);
    }

    #[test]
    fn symbol_from_str() {
        let s: Symbol = "EURUSD".into();
        assert_eq!(s.as_str(), "EURUSD");
    }

    #[test]
    fn points_add_sub() {
        let a: Points = 10.into();
        let b: Points = 3.into();
        assert_eq!((a + b).into_inner(), 13);
        assert_eq!((a - b).into_inner(), 7);
    }

    #[test]
    #[cfg(feature = "serde_json")]
    fn serde_round_trip() {
        let p: Price = Price::try_from(1.10).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: Price = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}

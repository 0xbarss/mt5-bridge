//! Project-wide constants and default tolerances for MT5 bridge operations.

/// Default tolerance for price equality checks (1e-9).
///
/// Used for: tick de-duplication, fill validation, position reconciliation,
/// order volume comparisons, and general price-level equality.
pub const PRICE_TOLERANCE: f64 = 1e-9;

/// Tolerance for timestamp equality checks (1e-6 seconds = 1 microsecond).
///
/// Used for: tick de-duplication, bar timestamp alignment.
pub const TIME_TOLERANCE: f64 = 1e-6;

/// Ultra-tight tolerance for high-precision calculations (1e-12).
pub const ULTRA_TOLERANCE: f64 = 1e-12;

/// Tick de-duplication price threshold (1e-9).
pub const DUP_PRICE_THRESHOLD: f64 = PRICE_TOLERANCE;

/// Tick de-duplication timestamp threshold (1e-6 seconds).
pub const DUP_TIME_THRESHOLD: f64 = TIME_TOLERANCE;

use crate::reconciliation::StrategyMagic;

/// Magic number used by the default EA configuration (`InpMagicNumber`).
pub const EA_DEFAULT_MAGIC: u64 = 20240101;

/// Strategy magic corresponding to [`EA_DEFAULT_MAGIC`].
pub const EA_DEFAULT_STRATEGY_MAGIC: StrategyMagic =
    match StrategyMagic::const_new(EA_DEFAULT_MAGIC) {
        Some(magic) => magic,
        None => unreachable!(),
    };

/// Default magic number for identifying orders/positions belonging to this bridge client.
#[deprecated(
    since = "0.4.0",
    note = "use EA_DEFAULT_MAGIC or an explicit strategy magic"
)]
pub const DEFAULT_MAGIC: u64 = 424242;

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::const_assert;

    const_assert!(PRICE_TOLERANCE > 0.0);
    const_assert!(TIME_TOLERANCE > 0.0);
    const_assert!(ULTRA_TOLERANCE > 0.0);

    #[test]
    #[allow(deprecated)]
    fn test_tolerance_values() {
        assert_eq!(DUP_PRICE_THRESHOLD, PRICE_TOLERANCE);
        assert_eq!(DUP_TIME_THRESHOLD, TIME_TOLERANCE);
        assert_eq!(EA_DEFAULT_MAGIC, 20240101);
        assert_eq!(EA_DEFAULT_STRATEGY_MAGIC.get(), EA_DEFAULT_MAGIC);
        assert_eq!(EA_DEFAULT_STRATEGY_MAGIC, EA_DEFAULT_MAGIC);
        assert_eq!(DEFAULT_MAGIC, 424242);
    }
}

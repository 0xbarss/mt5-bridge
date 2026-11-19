use crate::constants::PRICE_TOLERANCE;
use crate::ffi::Mt5SymInfo;
use serde::{Deserialize, Serialize};

/// Specification and trading parameters for a symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SymbolInfo {
    pub symbol: String,
    pub point: f64,
    pub tick_value: f64,
    pub tick_size: f64,
    pub lot_step: f64,
    pub min_lot: f64,
    pub max_lot: f64,
    pub spread: f64,
    pub digits: u32,
    /// The last bid the terminal reported. `0` when the tick was unavailable.
    pub bid: f64,
    /// The last ask the terminal reported. `0` when the tick was unavailable.
    pub ask: f64,
}

impl SymbolInfo {
    pub(crate) fn from_raw(symbol: &str, raw: Mt5SymInfo) -> Self {
        Self {
            symbol: symbol.to_string(),
            point: raw.point,
            tick_value: raw.tick_value,
            tick_size: raw.tick_size,
            lot_step: raw.lot_step,
            min_lot: raw.min_lot,
            max_lot: raw.max_lot,
            spread: raw.spread,
            digits: if raw.digits >= 0 {
                raw.digits as u32
            } else {
                0
            },
            bid: raw.bid,
            ask: raw.ask,
        }
    }

    /// Calculates the number of decimal places for a given lot step (e.g. 0.01 -> 2, 0.001 -> 3, 1.0 -> 0).
    pub fn calculate_lot_digits(lot_step: f64) -> u32 {
        if lot_step <= 0.0 || !lot_step.is_finite() {
            return 2;
        }
        let mut step = lot_step;
        let mut digits = 0;
        while digits < 8 {
            let diff = (step - step.round()).abs();
            if diff < 1e-4 {
                break;
            }
            step *= 10.0;
            digits += 1;
        }
        digits
    }

    /// Number of decimal digits for trade volumes on this symbol based on `lot_step`.
    pub fn lot_digits(&self) -> u32 {
        Self::calculate_lot_digits(self.lot_step)
    }

    /// Returns `true` if `min_lot` is an integer multiple of `lot_step` from zero.
    /// When `false`, the symbol uses a grid offset from `min_lot` (`min_lot + k * lot_step`).
    pub fn is_min_lot_zero_aligned(&self) -> bool {
        if self.lot_step <= 0.0 || self.min_lot <= 0.0 {
            return true;
        }
        let steps = self.min_lot / self.lot_step;
        (steps - steps.round()).abs() <= 1e-4
    }

    /// Normalizes and clamps a lot size according to `lot_step`, `min_lot`, and `max_lot`.
    ///
    /// Handles both zero-offset and `min_lot`-offset lot step grids, cleans up
    /// floating-point binary representation artifacts using [`lot_digits`],
    /// and prevents risk inflation by returning 0.0 if the lot is strictly below `min_lot`
    /// (with tolerance for floating-point underflow).
    pub fn round_lot(&self, lot: f64) -> f64 {
        if !lot.is_finite() || lot <= 0.0 || self.lot_step <= 0.0 {
            return 0.0;
        }
        // Protect against risk inflation when lot is below min_lot.
        // Tolerates 1e-7 floating-point underflow (e.g. 0.009999999999999998 for 0.01 min_lot).
        if self.min_lot > 0.0 && lot < self.min_lot - 1e-7 {
            return 0.0;
        }

        let rounded = if self.min_lot > 0.0 && !self.is_min_lot_zero_aligned() {
            let steps = ((lot - self.min_lot + PRICE_TOLERANCE) / self.lot_step).round();
            self.min_lot + steps.max(0.0) * self.lot_step
        } else {
            let steps = ((lot + PRICE_TOLERANCE) / self.lot_step).round();
            steps * self.lot_step
        };

        let clamped = if self.max_lot > 0.0 {
            rounded.max(self.min_lot).min(self.max_lot)
        } else {
            rounded.max(self.min_lot)
        };

        let digits = self.lot_digits();
        let factor = 10f64.powi(digits as i32);
        (clamped * factor).round() / factor
    }

    /// Floors a lot size down to the nearest valid step according to `lot_step`, `min_lot`, and `max_lot`.
    ///
    /// Unlike [`round_lot`], `floor_lot` will **never round up**, ensuring that strict risk limits
    /// (e.g. max dollar risk sizing) are never exceeded. Returns 0.0 if the floored volume is below `min_lot`.
    pub fn floor_lot(&self, lot: f64) -> f64 {
        if !lot.is_finite() || lot <= 0.0 || self.lot_step <= 0.0 {
            return 0.0;
        }
        if self.min_lot > 0.0 && lot < self.min_lot - 1e-7 {
            return 0.0;
        }

        let floored = if self.min_lot > 0.0 && !self.is_min_lot_zero_aligned() {
            let steps = ((lot - self.min_lot + PRICE_TOLERANCE) / self.lot_step).floor();
            self.min_lot + steps.max(0.0) * self.lot_step
        } else {
            let steps = ((lot + PRICE_TOLERANCE) / self.lot_step).floor();
            steps * self.lot_step
        };

        if self.min_lot > 0.0 && floored < self.min_lot - 1e-7 {
            return 0.0;
        }

        let clamped = if self.max_lot > 0.0 {
            floored.max(self.min_lot).min(self.max_lot)
        } else {
            floored.max(self.min_lot)
        };

        let digits = self.lot_digits();
        let factor = 10f64.powi(digits as i32);
        (clamped * factor).round() / factor
    }

    /// Ceils a lot size up to the nearest valid step according to `lot_step`, `min_lot`, and `max_lot`.
    pub fn ceil_lot(&self, lot: f64) -> f64 {
        if !lot.is_finite() || lot <= 0.0 || self.lot_step <= 0.0 {
            return 0.0;
        }

        let ceiled = if self.min_lot > 0.0 && !self.is_min_lot_zero_aligned() {
            let steps = ((lot - self.min_lot - PRICE_TOLERANCE) / self.lot_step).ceil();
            self.min_lot + steps.max(0.0) * self.lot_step
        } else {
            let steps = ((lot - PRICE_TOLERANCE) / self.lot_step).ceil();
            steps * self.lot_step
        };

        let clamped = if self.max_lot > 0.0 {
            ceiled.max(self.min_lot).min(self.max_lot)
        } else {
            ceiled.max(self.min_lot)
        };

        let digits = self.lot_digits();
        let factor = 10f64.powi(digits as i32);
        (clamped * factor).round() / factor
    }

    /// Checks if a lot size satisfies broker minimum, maximum, and lot step constraints.
    /// Supports both zero-based step multiples and `min_lot`-offset grids, with floating-point tolerance.
    pub fn is_valid_lot(&self, lot: f64) -> bool {
        if !lot.is_finite() || lot <= 0.0 {
            return false;
        }
        if self.min_lot > 0.0 && lot < self.min_lot - 1e-7 {
            return false;
        }
        if self.max_lot > 0.0 && lot > self.max_lot + 1e-7 {
            return false;
        }
        if self.lot_step > 0.0 {
            let steps_zero = lot / self.lot_step;
            let steps_min = if self.min_lot > 0.0 {
                (lot - self.min_lot) / self.lot_step
            } else {
                steps_zero
            };
            let ok_zero = (steps_zero - steps_zero.round()).abs() <= 1e-4;
            let ok_min = (steps_min - steps_min.round()).abs() <= 1e-4;
            if !ok_zero && !ok_min {
                return false;
            }
        }
        true
    }

    /// Monetary value of a 1-point price move for a given lot volume.
    /// Properly scales when `tick_size` differs from `point` (e.g. for CFDs, indices, commodities).
    pub fn point_value(&self, volume: f64) -> f64 {
        if self.tick_size > 0.0 {
            (self.point / self.tick_size) * self.tick_value * volume
        } else {
            self.tick_value * volume
        }
    }

    /// Price move to account currency multiplier per unit volume (`tick_value / tick_size`).
    pub fn price_multiplier(&self) -> f64 {
        let denom = if self.tick_size > 0.0 {
            self.tick_size
        } else if self.point > 0.0 {
            self.point
        } else {
            0.0
        };
        if denom > 0.0 {
            self.tick_value / denom
        } else {
            1.0
        }
    }

    /// Normalizes a price according to `tick_size` and `digits`.
    pub fn round_price(&self, price: f64) -> f64 {
        if self.tick_size > 0.0 {
            let steps = (price / self.tick_size).round();
            let rounded = steps * self.tick_size;
            let factor = 10f64.powi(self.digits as i32);
            (rounded * factor).round() / factor
        } else if self.digits > 0 {
            let factor = 10f64.powi(self.digits as i32);
            (price * factor).round() / factor
        } else {
            price
        }
    }
}

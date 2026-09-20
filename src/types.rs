use crate::error::mt5_retcode_description;
use crate::ffi::{Mt5Order, Mt5Position, Mt5Rate, Mt5SymInfo, Mt5Tick, Mt5TradeResult};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Supported MetaTrader 5 chart timeframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Timeframe {
    M1,
    M2,
    M3,
    M5,
    M6,
    M10,
    M12,
    M15,
    M20,
    M30,
    H1,
    H2,
    H3,
    H4,
    H6,
    H8,
    H12,
    D1,
    W1,
    MN1,
}

impl Timeframe {
    /// Return the corresponding MT5 `ENUM_TIMEFRAMES` integer constant.
    pub fn to_mt5_const(self) -> i32 {
        match self {
            Timeframe::M1 => 1,
            Timeframe::M2 => 2,
            Timeframe::M3 => 3,
            Timeframe::M5 => 5,
            Timeframe::M6 => 6,
            Timeframe::M10 => 10,
            Timeframe::M12 => 12,
            Timeframe::M15 => 15,
            Timeframe::M20 => 20,
            Timeframe::M30 => 30,
            Timeframe::H1 => 16385,
            Timeframe::H2 => 16386,
            Timeframe::H3 => 16387,
            Timeframe::H4 => 16388,
            Timeframe::H6 => 16390,
            Timeframe::H8 => 16392,
            Timeframe::H12 => 16396,
            Timeframe::D1 => 16408,
            Timeframe::W1 => 32769,
            Timeframe::MN1 => 49153,
        }
    }

    /// Duration of the timeframe bar in seconds.
    pub fn seconds(self) -> i64 {
        match self {
            Timeframe::M1 => 60,
            Timeframe::M2 => 120,
            Timeframe::M3 => 180,
            Timeframe::M5 => 300,
            Timeframe::M6 => 360,
            Timeframe::M10 => 600,
            Timeframe::M12 => 720,
            Timeframe::M15 => 900,
            Timeframe::M20 => 1200,
            Timeframe::M30 => 1800,
            Timeframe::H1 => 3600,
            Timeframe::H2 => 7200,
            Timeframe::H3 => 10800,
            Timeframe::H4 => 14400,
            Timeframe::H6 => 21600,
            Timeframe::H8 => 28800,
            Timeframe::H12 => 43200,
            Timeframe::D1 => 86400,
            Timeframe::W1 => 604800,
            Timeframe::MN1 => 2592000,
        }
    }

    /// Returns true for timeframes whose calendar length is variable (weeks and months).
    pub fn is_calendar_interval(self) -> bool {
        matches!(self, Timeframe::W1 | Timeframe::MN1)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Timeframe::M1 => "M1",
            Timeframe::M2 => "M2",
            Timeframe::M3 => "M3",
            Timeframe::M5 => "M5",
            Timeframe::M6 => "M6",
            Timeframe::M10 => "M10",
            Timeframe::M12 => "M12",
            Timeframe::M15 => "M15",
            Timeframe::M20 => "M20",
            Timeframe::M30 => "M30",
            Timeframe::H1 => "H1",
            Timeframe::H2 => "H2",
            Timeframe::H3 => "H3",
            Timeframe::H4 => "H4",
            Timeframe::H6 => "H6",
            Timeframe::H8 => "H8",
            Timeframe::H12 => "H12",
            Timeframe::D1 => "D1",
            Timeframe::W1 => "W1",
            Timeframe::MN1 => "MN1",
        }
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for Timeframe {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "M1" | "1M" => Ok(Timeframe::M1),
            "M2" | "2M" => Ok(Timeframe::M2),
            "M3" | "3M" => Ok(Timeframe::M3),
            "M5" | "5M" => Ok(Timeframe::M5),
            "M6" | "6M" => Ok(Timeframe::M6),
            "M10" | "10M" => Ok(Timeframe::M10),
            "M12" | "12M" => Ok(Timeframe::M12),
            "M15" | "15M" => Ok(Timeframe::M15),
            "M20" | "20M" => Ok(Timeframe::M20),
            "M30" | "30M" => Ok(Timeframe::M30),
            "H1" | "1H" => Ok(Timeframe::H1),
            "H2" | "2H" => Ok(Timeframe::H2),
            "H3" | "3H" => Ok(Timeframe::H3),
            "H4" | "4H" => Ok(Timeframe::H4),
            "H6" | "6H" => Ok(Timeframe::H6),
            "H8" | "8H" => Ok(Timeframe::H8),
            "H12" | "12H" => Ok(Timeframe::H12),
            "D1" | "1D" => Ok(Timeframe::D1),
            "W1" | "1W" => Ok(Timeframe::W1),
            "MN1" | "1MN" | "1MO" => Ok(Timeframe::MN1),
            _ => Err(format!("Unknown timeframe string: '{s}'")),
        }
    }
}

/// MetaTrader 5 account balance, equity, and margin details.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct AccountInfo {
    /// Account balance in deposit currency.
    pub balance: f64,
    /// Account equity in deposit currency.
    pub equity: f64,
    /// Currently reserved margin.
    pub margin: f64,
    /// Free margin available for opening new positions.
    pub free_margin: f64,
}

impl AccountInfo {
    /// Floating profit or loss (`equity - balance`).
    pub fn profit(&self) -> f64 {
        self.equity - self.balance
    }

    /// Current margin level percentage (`equity / margin * 100.0`), or `None` if margin is zero.
    pub fn margin_level(&self) -> Option<f64> {
        if self.margin > 0.0 {
            Some((self.equity / self.margin) * 100.0)
        } else {
            None
        }
    }
}

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
            let steps = ((lot - self.min_lot + 1e-9) / self.lot_step).round();
            self.min_lot + steps.max(0.0) * self.lot_step
        } else {
            let steps = ((lot + 1e-9) / self.lot_step).round();
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
            let steps = ((lot - self.min_lot + 1e-9) / self.lot_step).floor();
            self.min_lot + steps.max(0.0) * self.lot_step
        } else {
            let steps = ((lot + 1e-9) / self.lot_step).floor();
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
            let steps = ((lot - self.min_lot - 1e-9) / self.lot_step).ceil();
            self.min_lot + steps.max(0.0) * self.lot_step
        } else {
            let steps = ((lot - 1e-9) / self.lot_step).ceil();
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

/// Real-time quote tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    pub symbol: String,
    /// Time of the last quote in milliseconds (UTC unix timestamp).
    pub time_msc: i64,
    /// Time of the last quote in seconds (UTC unix timestamp).
    pub time: i64,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume: u64,
    pub flags: u32,
}

impl Tick {
    pub(crate) fn from_raw(symbol: &str, raw: Mt5Tick) -> Self {
        Self {
            symbol: symbol.to_string(),
            time_msc: raw.time,
            time: raw.time / 1000,
            bid: raw.bid,
            ask: raw.ask,
            last: raw.last,
            volume: raw.volume,
            flags: raw.flags,
        }
    }

    /// Spread in quote currency (`ask - bid`).
    pub fn spread(&self) -> f64 {
        self.ask - self.bid
    }

    /// Mid-price (`(ask + bid) / 2.0`).
    pub fn mid(&self) -> f64 {
        (self.ask + self.bid) / 2.0
    }
}

/// Raw rate structure returned by `CopyRates`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Rate {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub spread: i32,
    pub real_volume: i64,
}

impl From<Mt5Rate> for Rate {
    fn from(r: Mt5Rate) -> Self {
        Self {
            time: r.time,
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
            volume: r.volume,
            spread: r.spread,
            real_volume: r._rv,
        }
    }
}

/// Clean OHLCV bar representation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Bar {
    pub fn new(time: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Self {
            time,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    /// Mid-point price: `(high + low) / 2.0`.
    pub fn mid(&self) -> f64 {
        (self.high + self.low) / 2.0
    }

    /// Typical price: `(high + low + close) / 3.0`.
    pub fn typical_price(&self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }

    /// High - Low range.
    pub fn range(&self) -> f64 {
        self.high - self.low
    }

    /// Absolute candle body size: `|close - open|`.
    pub fn body(&self) -> f64 {
        (self.close - self.open).abs()
    }

    /// True range compared to the previous bar's close.
    pub fn true_range(&self, prev_close: f64) -> f64 {
        let hl = self.high - self.low;
        let hc = (self.high - prev_close).abs();
        let lc = (self.low - prev_close).abs();
        hl.max(hc).max(lc)
    }

    pub fn is_bullish(&self) -> bool {
        self.close > self.open
    }

    pub fn is_bearish(&self) -> bool {
        self.close < self.open
    }
}

impl From<Rate> for Bar {
    fn from(r: Rate) -> Self {
        Self {
            time: r.time,
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
            volume: r.volume as f64,
        }
    }
}

/// Order type matching MetaTrader 5 `ENUM_ORDER_TYPE`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Buy = 0,
    Sell = 1,
    BuyLimit = 2,
    SellLimit = 3,
    BuyStop = 4,
    SellStop = 5,
}

impl OrderType {
    pub fn is_buy(self) -> bool {
        matches!(
            self,
            OrderType::Buy | OrderType::BuyLimit | OrderType::BuyStop
        )
    }

    pub fn is_sell(self) -> bool {
        matches!(
            self,
            OrderType::Sell | OrderType::SellLimit | OrderType::SellStop
        )
    }
}

/// Parameters for placing an order via `order_send`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub order_type: OrderType,
    pub volume: f64,
    pub price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub comment: String,
    pub deviation: Option<u32>,
    pub expiration: Option<i64>,
    pub magic: Option<u64>,
    pub client_order_id: Option<String>,
}

impl OrderRequest {
    /// Create a new market Buy order request.
    pub fn buy(symbol: impl Into<String>, volume: f64) -> Self {
        Self {
            symbol: symbol.into(),
            order_type: OrderType::Buy,
            volume,
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Create a new market Sell order request.
    pub fn sell(symbol: impl Into<String>, volume: f64) -> Self {
        Self {
            symbol: symbol.into(),
            order_type: OrderType::Sell,
            volume,
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Create a new limit or pending order request.
    pub fn pending(
        symbol: impl Into<String>,
        order_type: OrderType,
        volume: f64,
        price: f64,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            order_type,
            volume,
            price,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    pub fn stop_loss(mut self, sl: f64) -> Self {
        self.stop_loss = sl;
        self
    }

    pub fn take_profit(mut self, tp: f64) -> Self {
        self.take_profit = tp;
        self
    }

    pub fn price(mut self, price: f64) -> Self {
        self.price = price;
        self
    }

    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = comment.into();
        self
    }

    pub fn deviation(mut self, deviation: u32) -> Self {
        self.deviation = Some(deviation);
        self
    }

    pub fn expiration(mut self, expiration: i64) -> Self {
        self.expiration = Some(expiration);
        self
    }

    pub fn magic(mut self, magic: u64) -> Self {
        self.magic = Some(magic);
        self
    }

    pub fn client_order_id(mut self, id: impl Into<String>) -> Self {
        self.client_order_id = Some(id.into());
        self
    }

    /// Formats the effective MT5 wire comment embedding client_order_id if present:
    /// e.g. "cid:<client_order_id>" or "cid:<client_order_id>:<comment>", truncated to 31 chars max.
    pub fn effective_comment(&self) -> String {
        match &self.client_order_id {
            Some(cid) if !cid.is_empty() => {
                if self.comment.is_empty() {
                    let s = format!("cid:{cid}");
                    if s.len() > 31 {
                        s[..31].to_string()
                    } else {
                        s
                    }
                } else {
                    let s = format!("cid:{cid}:{}", self.comment);
                    if s.len() > 31 {
                        s[..31].to_string()
                    } else {
                        s
                    }
                }
            }
            _ => {
                if self.comment.len() > 31 {
                    self.comment[..31].to_string()
                } else {
                    self.comment.clone()
                }
            }
        }
    }

    /// Validates the order request parameters before submitting to the MT5 bridge.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.symbol.trim().is_empty() {
            return Err(crate::error::Mt5Error::Other(
                "OrderRequest symbol cannot be empty".to_string(),
            ));
        }
        if self.volume <= 0.0 || self.volume.is_nan() || self.volume.is_infinite() {
            return Err(crate::error::Mt5Error::Other(format!(
                "Invalid order volume {}: volume must be a positive finite number",
                self.volume
            )));
        }
        if self.price < 0.0 || self.price.is_nan() || self.price.is_infinite() {
            return Err(crate::error::Mt5Error::Other(format!(
                "Invalid order price {}: price cannot be negative or NaN",
                self.price
            )));
        }
        if self.stop_loss < 0.0 || self.stop_loss.is_nan() || self.stop_loss.is_infinite() {
            return Err(crate::error::Mt5Error::Other(format!(
                "Invalid stop loss {}: stop loss cannot be negative or NaN",
                self.stop_loss
            )));
        }
        if self.take_profit < 0.0 || self.take_profit.is_nan() || self.take_profit.is_infinite() {
            return Err(crate::error::Mt5Error::Other(format!(
                "Invalid take profit {}: take profit cannot be negative or NaN",
                self.take_profit
            )));
        }
        Ok(())
    }

    /// Validates the order request against live symbol specifications:
    /// minimum/maximum lot sizes, lot step alignment, and tick size alignment.
    pub fn validate_with_symbol(&self, info: &SymbolInfo) -> crate::error::Result<()> {
        self.validate()?;
        if !info.is_valid_lot(self.volume) {
            return Err(crate::error::Mt5Error::Other(format!(
                "Order volume {} does not satisfy symbol {} lot constraints (min={}, max={}, step={})",
                self.volume, self.symbol, info.min_lot, info.max_lot, info.lot_step
            )));
        }
        if info.tick_size > 0.0 {
            if self.price > 0.0 {
                let steps = self.price / info.tick_size;
                if (steps - steps.round()).abs() > 1e-4 {
                    return Err(crate::error::Mt5Error::Other(format!(
                        "Order price {} is not aligned to tick size {}",
                        self.price, info.tick_size
                    )));
                }
            }
            if self.stop_loss > 0.0 {
                let steps = self.stop_loss / info.tick_size;
                if (steps - steps.round()).abs() > 1e-4 {
                    return Err(crate::error::Mt5Error::Other(format!(
                        "Stop loss {} is not aligned to tick size {}",
                        self.stop_loss, info.tick_size
                    )));
                }
            }
            if self.take_profit > 0.0 {
                let steps = self.take_profit / info.tick_size;
                if (steps - steps.round()).abs() > 1e-4 {
                    return Err(crate::error::Mt5Error::Other(format!(
                        "Take profit {} is not aligned to tick size {}",
                        self.take_profit, info.tick_size
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Trade execution status classification (high-level convenience classification).
///
/// **Authoritative Semantics:**
/// The authoritative field for MT5 execution outcome is always [`TradeResult::retcode`].
/// This enum provides an ergonomic classification of common MT5 outcomes:
/// - `Filled`: Market order executed immediately with a deal ticket (`TRADE_RETCODE_DONE` = 10009, `deal > 0`).
/// - `Placed`: Pending order accepted and currently working in the terminal (`TRADE_RETCODE_PLACED` = 10008,
///   or 10009 with `deal == 0` and `order > 0`).
/// - `PartiallyFilled`: Order partially executed (`TRADE_RETCODE_DONE_PARTIAL` = 10010).
/// - `Rejected`: Order rejected or failed (any error retcode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeStatus {
    /// Request completed in full (`TRADE_RETCODE_DONE` = 10009 with `deal > 0`).
    Filled,
    /// Pending order placed and working (`TRADE_RETCODE_PLACED` = 10008 or 10009 with `deal == 0`).
    Placed,
    /// Only part of the requested volume was filled (`TRADE_RETCODE_DONE_PARTIAL` = 10010).
    PartiallyFilled,
    /// Order was rejected or failed.
    Rejected,
}

/// Result returned from an order placement or closure.
///
/// **Authoritative Outcome:**
/// The authoritative status of the order is [`retcode`](Self::retcode).
/// Use [`is_deal()`](Self::is_deal) to check if a deal was executed immediately,
/// [`is_working_order()`](Self::is_working_order) to check if a pending order is active,
/// and [`has_position()`](Self::has_position) to check if a position ticket was assigned.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TradeResult {
    /// MT5 Trade execution return code (e.g. 10009 for `TRADE_RETCODE_DONE`).
    /// This is the authoritative raw status code returned by MetaTrader 5.
    pub retcode: u32,
    /// Deal ticket number if a deal was executed (0 for pending orders).
    pub deal: u64,
    /// Order ticket number.
    pub order: u64,
    /// Position ticket number associated with the trade (0 if pending or unknown).
    pub position: u64,
    /// Executed trade volume.
    pub volume: f64,
    /// Execution price.
    pub price: f64,
}

impl TradeResult {
    pub fn from_raw(raw: Mt5TradeResult) -> Self {
        Self {
            retcode: raw.retcode,
            deal: raw.deal,
            order: raw.order,
            position: raw.position,
            volume: raw.volume,
            price: raw.price,
        }
    }

    /// Classified status of the trade execution.
    pub fn status(&self) -> TradeStatus {
        match self.retcode {
            10009 => {
                if self.deal > 0 {
                    TradeStatus::Filled
                } else if self.order > 0 {
                    TradeStatus::Placed
                } else {
                    TradeStatus::Filled
                }
            }
            10008 => TradeStatus::Placed,
            10010 => TradeStatus::PartiallyFilled,
            _ => TradeStatus::Rejected,
        }
    }

    /// Returns `true` if the order was successfully completed, placed, or partially filled.
    pub fn is_success(&self) -> bool {
        matches!(
            self.status(),
            TradeStatus::Filled | TradeStatus::Placed | TradeStatus::PartiallyFilled
        )
    }

    /// Returns `true` if the order was executed in full as an immediate deal.
    pub fn is_filled(&self) -> bool {
        self.status() == TradeStatus::Filled
    }

    /// Returns `true` if a pending order was placed and is working.
    pub fn is_placed(&self) -> bool {
        self.status() == TradeStatus::Placed
    }

    /// Returns `true` if only part of the requested volume was filled.
    pub fn is_partially_filled(&self) -> bool {
        self.status() == TradeStatus::PartiallyFilled
    }

    /// Returns `true` if a deal was executed immediately (`deal > 0`).
    pub fn is_deal(&self) -> bool {
        self.deal > 0
    }

    /// Returns `true` if this result represents a working pending order (`deal == 0 && order > 0`).
    pub fn is_working_order(&self) -> bool {
        self.deal == 0 && self.order > 0
    }

    /// Returns `true` if a non-zero position ticket is present.
    pub fn has_position(&self) -> bool {
        self.position > 0
    }

    /// Human-readable explanation of the return code.
    pub fn description(&self) -> &'static str {
        mt5_retcode_description(self.retcode)
    }
}

/// Detailed information for an active open position in MetaTrader 5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub ticket: u64,
    pub time: i64,
    pub position_type: OrderType,
    pub magic: u64,
    pub volume: f64,
    pub price_open: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub price_current: f64,
    pub profit: f64,
    pub swap: f64,
    pub symbol: String,
    pub comment: String,
}

impl Position {
    pub fn from_raw(raw: Mt5Position) -> Self {
        let sym_len = raw
            .symbol
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(raw.symbol.len());
        let sym = String::from_utf8_lossy(&raw.symbol[..sym_len])
            .trim()
            .to_string();

        let cmt_len = raw
            .comment
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(raw.comment.len());
        let cmt = String::from_utf8_lossy(&raw.comment[..cmt_len])
            .trim()
            .to_string();

        let ptype = if raw.position_type == 1 {
            OrderType::Sell
        } else {
            OrderType::Buy
        };

        Self {
            ticket: raw.ticket,
            time: raw.time,
            position_type: ptype,
            magic: raw.magic,
            volume: raw.volume,
            price_open: raw.price_open,
            stop_loss: raw.sl,
            take_profit: raw.tp,
            price_current: raw.price_current,
            profit: raw.profit,
            swap: raw.swap,
            symbol: sym,
            comment: cmt,
        }
    }

    pub fn is_buy(&self) -> bool {
        self.position_type.is_buy()
    }

    pub fn is_sell(&self) -> bool {
        self.position_type.is_sell()
    }

    pub fn matches_magic(&self, magic: u64) -> bool {
        self.magic == magic
    }
}

/// Detailed information for an active working pending order in MetaTrader 5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkingOrder {
    pub ticket: u64,
    pub time_setup: i64,
    pub order_type: OrderType,
    pub magic: u64,
    pub volume_initial: f64,
    pub volume_current: f64,
    pub price_open: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub price_current: f64,
    pub symbol: String,
    pub comment: String,
}

impl WorkingOrder {
    pub fn from_raw(raw: Mt5Order) -> Self {
        let sym_len = raw
            .symbol
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(raw.symbol.len());
        let sym = String::from_utf8_lossy(&raw.symbol[..sym_len])
            .trim()
            .to_string();

        let cmt_len = raw
            .comment
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(raw.comment.len());
        let cmt = String::from_utf8_lossy(&raw.comment[..cmt_len])
            .trim()
            .to_string();

        let otype = match raw.order_type {
            2 => OrderType::BuyLimit,
            3 => OrderType::SellLimit,
            4 => OrderType::BuyStop,
            5 => OrderType::SellStop,
            1 => OrderType::Sell,
            _ => OrderType::Buy,
        };

        Self {
            ticket: raw.ticket,
            time_setup: raw.time_setup,
            order_type: otype,
            magic: raw.magic,
            volume_initial: raw.volume_initial,
            volume_current: raw.volume_current,
            price_open: raw.price_open,
            stop_loss: raw.sl,
            take_profit: raw.tp,
            price_current: raw.price_current,
            symbol: sym,
            comment: cmt,
        }
    }

    pub fn matches_magic(&self, magic: u64) -> bool {
        self.magic == magic
    }
}

/// Complete lifecycle state of an order in the execution state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderState {
    /// Initial state before network transmission.
    Created,
    /// Actively transmitting or awaiting confirmation.
    Submitting,
    /// Pending order accepted and working in the MT5 order book.
    Accepted,
    /// Order partially executed; working remainder remains.
    PartiallyFilled,
    /// Order completely executed.
    Filled,
    /// Order was cancelled or expired.
    Cancelled,
    /// Order was rejected by the bridge or broker.
    Rejected,
    /// Ambiguous outcome (e.g. timeout / disconnect after submission).
    /// Requires reconciliation against broker state before any retry!
    Unknown,
    /// Order confirmed and reconstructed via reconciliation.
    Reconciled,
}

/// Order tracker maintaining full execution lifecycle, volume accounting, and reconciliation state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedOrder {
    pub client_order_id: String,
    pub symbol: String,
    pub order_type: OrderType,
    pub requested_volume: f64,
    pub filled_volume: f64,
    pub remaining_volume: f64,
    pub average_price: f64,
    pub order_ticket: u64,
    pub deal_ticket: u64,
    pub position_ticket: u64,
    pub state: OrderState,
    pub magic: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub error_message: Option<String>,
}

impl TrackedOrder {
    pub fn new(req: &OrderRequest, client_order_id: impl Into<String>) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            client_order_id: client_order_id.into(),
            symbol: req.symbol.clone(),
            order_type: req.order_type,
            requested_volume: req.volume,
            filled_volume: 0.0,
            remaining_volume: req.volume,
            average_price: 0.0,
            order_ticket: 0,
            deal_ticket: 0,
            position_ticket: 0,
            state: OrderState::Created,
            magic: req.magic.unwrap_or(0),
            created_at: now,
            updated_at: now,
            error_message: None,
        }
    }

    /// Mark the order as actively being submitted over the IPC bridge.
    pub fn mark_submitting(&mut self) {
        self.state = OrderState::Submitting;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Update order state from a synchronous TradeResult response.
    pub fn update_from_trade_result(&mut self, res: &TradeResult) {
        self.updated_at = chrono::Utc::now().timestamp();
        self.order_ticket = res.order;
        self.deal_ticket = res.deal;
        if res.position > 0 {
            self.position_ticket = res.position;
        }

        match res.status() {
            TradeStatus::Filled => {
                self.filled_volume = res.volume;
                self.remaining_volume = (self.requested_volume - res.volume).max(0.0);
                self.average_price = res.price;
                self.state = OrderState::Filled;
            }
            TradeStatus::Placed => {
                self.state = OrderState::Accepted;
            }
            TradeStatus::PartiallyFilled => {
                self.filled_volume = res.volume;
                self.remaining_volume = (self.requested_volume - res.volume).max(0.0);
                self.average_price = res.price;
                self.state = OrderState::PartiallyFilled;
            }
            TradeStatus::Rejected => {
                self.state = OrderState::Rejected;
                self.error_message = Some(res.description().to_string());
            }
        }
    }

    /// Mark order status as Unknown when network transmission is ambiguous (e.g. pipe broken during response).
    pub fn mark_unknown(&mut self, reason: impl Into<String>) {
        self.state = OrderState::Unknown;
        self.error_message = Some(reason.into());
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Update state when confirmed active or filled via position reconciliation.
    pub fn reconcile_with_position(&mut self, pos: &Position) {
        self.position_ticket = pos.ticket;
        self.filled_volume = pos.volume;
        self.remaining_volume = (self.requested_volume - pos.volume).max(0.0);
        self.average_price = pos.price_open;
        self.state = OrderState::Reconciled;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Update state when confirmed active via working pending order reconciliation.
    pub fn reconcile_with_working_order(&mut self, ord: &WorkingOrder) {
        self.order_ticket = ord.ticket;
        self.filled_volume = ord.volume_initial - ord.volume_current;
        self.remaining_volume = ord.volume_current;
        self.state = OrderState::Accepted;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Returns `true` if this order has reached a final state (Filled, Cancelled, Rejected).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            OrderState::Filled | OrderState::Cancelled | OrderState::Rejected
        )
    }

    /// Returns `true` if this order is still open or working (Created, Submitting, Accepted, PartiallyFilled, Unknown).
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

/// Converts a floating-point price into an integer number of price ticks based on tick size.
pub fn price_to_ticks(price: f64, tick_size: f64) -> i64 {
    if tick_size <= 0.0 || !price.is_finite() {
        0
    } else {
        ((price / tick_size) + 1e-9).round() as i64
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
    let sl_ticks = if is_buy {
        entry_ticks - distance_ticks.abs()
    } else {
        entry_ticks + distance_ticks.abs()
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
    let tp_ticks = if is_buy {
        entry_ticks + distance_ticks.abs()
    } else {
        entry_ticks - distance_ticks.abs()
    };
    ticks_to_price(tp_ticks, tick_size, digits)
}

/// Detailed historical data result with completeness tracking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryResult {
    /// Retrieved historical rates.
    pub rates: Vec<Rate>,
    /// Whether all requested chunks were successfully retrieved without error or missing ranges.
    pub complete: bool,
    /// Time ranges `(start, end)` that failed or were missing from broker history.
    pub missing_ranges: Vec<(i64, i64)>,
}

impl HistoryResult {
    /// Returns `true` if all requested chunks were retrieved with zero missing ranges.
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.complete && self.missing_ranges.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeframe_constants() {
        assert_eq!(Timeframe::M1.to_mt5_const(), 1);
        assert_eq!(Timeframe::M15.to_mt5_const(), 15);
        assert_eq!(Timeframe::H1.to_mt5_const(), 16385);
        assert_eq!(Timeframe::D1.to_mt5_const(), 16408);
        assert_eq!(Timeframe::M1.seconds(), 60);
        assert_eq!(Timeframe::H1.seconds(), 3600);
    }

    #[test]
    fn test_timeframe_parsing() {
        assert_eq!("m15".parse::<Timeframe>().unwrap(), Timeframe::M15);
        assert_eq!("1h".parse::<Timeframe>().unwrap(), Timeframe::H1);
        assert_eq!("D1".parse::<Timeframe>().unwrap(), Timeframe::D1);
    }

    #[test]
    fn test_lot_rounding() {
        let sym = SymbolInfo {
            symbol: "EURUSD".to_string(),
            point: 0.00001,
            tick_value: 1.0,
            tick_size: 0.00001,
            lot_step: 0.01,
            min_lot: 0.01,
            max_lot: 100.0,
            spread: 1.2,
            digits: 5,
        };

        assert_eq!(sym.round_lot(0.004), 0.0);
        assert_eq!(sym.round_lot(0.126), 0.13);
        assert_eq!(sym.round_lot(150.0), 100.0);
        assert!(sym.is_valid_lot(0.12));
        assert!(!sym.is_valid_lot(0.004));

        // Float noise elimination (0.29 / 0.01)
        assert_eq!(sym.round_lot(0.29), 0.29);
        assert_eq!(sym.round_lot(0.07), 0.07);

        // Tolerance for floating-point underflow slightly below min_lot
        assert_eq!(sym.round_lot(0.009999999999999998), 0.01);
        assert!(sym.is_valid_lot(0.009999999999999998));

        // Floor lot (essential for risk management so risk is not exceeded)
        assert_eq!(sym.floor_lot(0.129), 0.12);
        assert_eq!(sym.floor_lot(0.009), 0.0);

        // Ceil lot
        assert_eq!(sym.ceil_lot(0.121), 0.13);
        assert_eq!(sym.ceil_lot(0.001), 0.01);
    }

    #[test]
    fn test_lot_rounding_min_offset_pairs() {
        // Many CFD/crypto/index pairs have min_lot that is NOT a multiple of lot_step
        // E.g., min_lot = 0.05, lot_step = 0.02 -> allowed: 0.05, 0.07, 0.09, 0.11...
        let sym = SymbolInfo {
            symbol: "CFD_INDEX".to_string(),
            point: 0.01,
            tick_value: 1.0,
            tick_size: 0.01,
            lot_step: 0.02,
            min_lot: 0.05,
            max_lot: 50.0,
            spread: 2.0,
            digits: 2,
        };

        assert!(!sym.is_min_lot_zero_aligned());
        assert_eq!(sym.lot_digits(), 2);

        // Minimum lot itself MUST be valid and round to itself
        assert!(sym.is_valid_lot(0.05));
        assert_eq!(sym.round_lot(0.05), 0.05);

        // Step increments from min_lot (0.05 + 0.02 = 0.07)
        assert!(sym.is_valid_lot(0.07));
        assert_eq!(sym.round_lot(0.07), 0.07);

        assert!(sym.is_valid_lot(0.09));
        assert_eq!(sym.round_lot(0.09), 0.09);

        // Rounding nearest to min_lot grid
        assert_eq!(sym.round_lot(0.06), 0.07);
        assert_eq!(sym.floor_lot(0.06), 0.05);
        assert_eq!(sym.ceil_lot(0.06), 0.07);

        // Invalid lot not on step grid
        assert!(!sym.is_valid_lot(0.065));

        // Sub-min lot returns 0.0
        assert_eq!(sym.round_lot(0.02), 0.0);
        assert_eq!(sym.floor_lot(0.04), 0.0);
    }

    #[test]
    fn test_bar_calculations() {
        let bar = Bar::new(1700000000, 1.1000, 1.1050, 1.0950, 1.1020, 500.0);
        assert!((bar.mid() - 1.1000).abs() < 1e-6);
        assert!((bar.typical_price() - ((1.1050 + 1.0950 + 1.1020) / 3.0)).abs() < 1e-6);
        assert!((bar.range() - 0.0100).abs() < 1e-6);
        assert!(bar.is_bullish());
        assert!(!bar.is_bearish());
    }

    #[test]
    fn test_account_info_helpers() {
        let acct = AccountInfo {
            balance: 10000.0,
            equity: 10500.0,
            margin: 2000.0,
            free_margin: 8500.0,
        };
        assert_eq!(acct.profit(), 500.0);
        assert_eq!(acct.margin_level(), Some(525.0));
    }

    #[test]
    fn test_order_request_builder() {
        let req = OrderRequest::buy("EURUSD", 0.5)
            .stop_loss(1.0850)
            .take_profit(1.0950)
            .comment("test_buy")
            .deviation(20)
            .expiration(1800000000)
            .magic(123456);

        assert_eq!(req.symbol, "EURUSD");
        assert_eq!(req.order_type, OrderType::Buy);
        assert_eq!(req.volume, 0.5);
        assert_eq!(req.stop_loss, 1.0850);
        assert_eq!(req.take_profit, 1.0950);
        assert_eq!(req.comment, "test_buy");
        assert_eq!(req.deviation, Some(20));
        assert_eq!(req.expiration, Some(1800000000));
        assert_eq!(req.magic, Some(123456));
    }

    #[test]
    fn test_point_value_scaling() {
        // Forex: point == tick_size == 0.00001
        let forex = SymbolInfo {
            symbol: "EURUSD".to_string(),
            point: 0.00001,
            tick_value: 1.0,
            tick_size: 0.00001,
            lot_step: 0.01,
            min_lot: 0.01,
            max_lot: 100.0,
            spread: 1.0,
            digits: 5,
        };
        assert!((forex.point_value(1.0) - 1.0).abs() < 1e-6);

        // Index / CFD: point is 0.01, but tick_size is 0.25, tick_value is 12.50
        let index = SymbolInfo {
            symbol: "US500".to_string(),
            point: 0.01,
            tick_value: 12.50,
            tick_size: 0.25,
            lot_step: 0.1,
            min_lot: 0.1,
            max_lot: 100.0,
            spread: 2.0,
            digits: 2,
        };
        // 1 point (0.01) is 0.01 / 0.25 = 0.04 ticks. 0.04 * 12.50 = 0.50 per lot.
        assert!((index.point_value(1.0) - 0.50).abs() < 1e-6);
        assert!((index.point_value(2.0) - 1.00).abs() < 1e-6);
    }

    #[test]
    fn test_price_rounding() {
        let sym = SymbolInfo {
            symbol: "US500".to_string(),
            point: 0.01,
            tick_value: 12.50,
            tick_size: 0.25,
            lot_step: 0.1,
            min_lot: 0.1,
            max_lot: 100.0,
            spread: 2.0,
            digits: 2,
        };
        assert_eq!(sym.round_price(5000.12), 5000.0);
        assert_eq!(sym.round_price(5000.13), 5000.25);
        assert_eq!(sym.round_price(5000.37), 5000.25);
        assert_eq!(sym.round_price(5000.38), 5000.50);
    }
}

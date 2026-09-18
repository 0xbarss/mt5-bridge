use crate::error::mt5_retcode_description;
use crate::ffi::{Mt5Rate, Mt5SymInfo, Mt5Tick, Mt5TradeResult};
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
            lot_step: raw.lot_step,
            min_lot: raw.min_lot,
            max_lot: raw.max_lot,
            spread: raw.spread,
            digits: if raw.digits >= 0 { raw.digits as u32 } else { 0 },
        }
    }

    /// Normalizes and clamps a lot size according to `lot_step`, `min_lot`, and `max_lot`.
    pub fn round_lot(&self, lot: f64) -> f64 {
        if self.lot_step <= 0.0 {
            return lot;
        }
        let steps = (lot / self.lot_step).round();
        let rounded = steps * self.lot_step;
        rounded.max(self.min_lot).min(self.max_lot)
    }

    /// Monetary value of a 1-point price move for a given lot volume.
    pub fn point_value(&self, volume: f64) -> f64 {
        self.tick_value * volume
    }
}

/// Real-time quote tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    pub symbol: String,
    /// Time of the last quote (UTC unix timestamp in seconds).
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
            time: raw.time,
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
        matches!(self, OrderType::Buy | OrderType::BuyLimit | OrderType::BuyStop)
    }

    pub fn is_sell(self) -> bool {
        matches!(self, OrderType::Sell | OrderType::SellLimit | OrderType::SellStop)
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
        }
    }

    /// Create a new limit or pending order request.
    pub fn pending(symbol: impl Into<String>, order_type: OrderType, volume: f64, price: f64) -> Self {
        Self {
            symbol: symbol.into(),
            order_type,
            volume,
            price,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
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
}

/// Result returned from an order placement or closure.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TradeResult {
    /// MT5 Trade execution return code (e.g. 10009 for `TRADE_RETCODE_DONE`).
    pub retcode: u32,
    /// Deal ticket number if a deal was executed.
    pub deal: u64,
    /// Order ticket number.
    pub order: u64,
    /// Executed trade volume.
    pub volume: f64,
    /// Execution price.
    pub price: f64,
}

impl TradeResult {
    pub(crate) fn from_raw(raw: Mt5TradeResult) -> Self {
        Self {
            retcode: raw.retcode,
            deal: raw.deal,
            order: raw.order,
            volume: raw.volume,
            price: raw.price,
        }
    }

    /// Returns `true` if the order was successfully completed or placed.
    pub fn is_success(&self) -> bool {
        self.retcode == 10009 || self.retcode == 10008
    }

    /// Human-readable explanation of the return code.
    pub fn description(&self) -> &'static str {
        mt5_retcode_description(self.retcode)
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
            lot_step: 0.01,
            min_lot: 0.01,
            max_lot: 100.0,
            spread: 1.2,
            digits: 5,
        };

        assert_eq!(sym.round_lot(0.004), 0.01);
        assert_eq!(sym.round_lot(0.126), 0.13);
        assert_eq!(sym.round_lot(150.0), 100.0);
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
            .comment("test_buy");

        assert_eq!(req.symbol, "EURUSD");
        assert_eq!(req.order_type, OrderType::Buy);
        assert_eq!(req.volume, 0.5);
        assert_eq!(req.stop_loss, 1.0850);
        assert_eq!(req.take_profit, 1.0950);
        assert_eq!(req.comment, "test_buy");
    }
}

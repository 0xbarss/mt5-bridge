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

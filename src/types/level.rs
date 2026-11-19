use serde::{Deserialize, Serialize};

/// Target price level specification for stop-loss and take-profit modifications.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Level {
    /// Keep the current level unchanged.
    Keep,
    /// Explicitly remove/clear the level (sets to 0.0).
    Clear,
    /// Update to an explicit price level.
    Set(f64),
}

impl Level {
    /// Convert to wire mode (0=keep, 1=clear, 2=set) and price value.
    pub fn to_wire(self) -> (u8, f64) {
        match self {
            Level::Keep => (0, 0.0),
            Level::Clear => (1, 0.0),
            Level::Set(v) => (2, v),
        }
    }
}

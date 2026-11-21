use crate::types::rate::Rate;
use serde::{Deserialize, Serialize};

/// Detailed historical data result with completeness tracking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryResult {
    /// Retrieved historical rates.
    pub rates: Vec<Rate>,
    /// Whether all requested chunks were successfully retrieved without error or missing ranges.
    pub complete: bool,
    /// Time ranges `(start, end)` that failed or were missing from broker history.
    pub missing_ranges: Vec<(i64, i64)>,
    /// Whether the historical data download was cancelled before completion.
    #[serde(default)]
    pub cancelled: bool,
}

impl HistoryResult {
    /// Returns `true` if all requested chunks were retrieved with zero missing ranges and no cancellation.
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.complete && !self.cancelled && self.missing_ranges.is_empty()
    }
}

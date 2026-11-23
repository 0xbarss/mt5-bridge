use crate::types::rate::Rate;
use serde::{Deserialize, Serialize};

/// Explicit lifecycle state of a historical-rate download.
///
/// Replaces the implicit `complete: bool` + `cancelled: bool` + `missing_ranges`
/// flag combination that callers had to recombine at every call site.
/// `HistoryState` is the single source of truth for download progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryState {
    /// No data retrieved yet; download has not started or is awaiting first chunk.
    Pending,
    /// Some rates retrieved but not all chunks completed; missing ranges remain.
    Partial,
    /// All requested chunks retrieved with zero missing ranges and no cancellation.
    Complete,
    /// Download was cancelled before completion; may contain partial rates.
    Cancelled,
}

impl HistoryState {
    /// Strict priority order for resolution: Cancelled > Complete > Partial > Pending.
    ///
    /// When multiple independent sources can describe the same download, this order
    /// defines which state wins. Cancelled is terminal and takes precedence over
    /// everything else; Complete outranks Partial and Pending.
    const PRIORITY: [HistoryState; 4] = [
        HistoryState::Cancelled,
        HistoryState::Complete,
        HistoryState::Partial,
        HistoryState::Pending,
    ];

    /// Returns the higher-priority of `self` and `other` per [`PRIORITY`](Self::PRIORITY).
    pub fn max(self, other: HistoryState) -> HistoryState {
        let self_idx = Self::PRIORITY.iter().position(|&s| s == self).unwrap();
        let other_idx = Self::PRIORITY.iter().position(|&s| s == other).unwrap();
        if self_idx < other_idx {
            self
        } else {
            other
        }
    }

    /// Pure transition: given the current state and an event, returns the next state
    /// with no side effects.
    ///
    /// Callers decide how to act on the returned state (log, warn, return error, etc.).
    /// The transition itself is side-effect free and trivially testable.
    pub fn transition(self, event: HistoryEvent) -> HistoryState {
        match (self, event) {
            // Cancelled is terminal — nothing can override it.
            (HistoryState::Cancelled, _) => HistoryState::Cancelled,
            // Complete is terminal once all chunks are done.
            (HistoryState::Complete, _) => HistoryState::Complete,
            // Pending stays pending on `AllChunksDone` with no data; moves to Partial on data.
            (HistoryState::Pending, HistoryEvent::RatesReceived) => HistoryState::Partial,
            (HistoryState::Pending, HistoryEvent::AllChunksDone) => HistoryState::Pending,
            // Partial stays partial on more data; becomes Complete when all chunks done.
            (HistoryState::Partial, HistoryEvent::RatesReceived) => HistoryState::Partial,
            (HistoryState::Partial, HistoryEvent::AllChunksDone) => HistoryState::Complete,
            // Any state transitions to Cancelled on cancel.
            (_, HistoryEvent::Cancelled) => HistoryState::Cancelled,
        }
    }

    /// Returns `true` if this is a terminal state (no further transitions possible).
    pub fn is_terminal(self) -> bool {
        matches!(self, HistoryState::Complete | HistoryState::Cancelled)
    }
}

/// Events that drive the [`HistoryState`] transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryEvent {
    /// A chunk of rates was received; more chunks may follow.
    RatesReceived,
    /// All requested chunks have been processed; no more data expected.
    AllChunksDone,
    /// The download was cancelled by the caller.
    Cancelled,
}

/// Detailed historical data result with explicit lifecycle state.
///
/// `state` is the single source of truth; `missing_ranges` provides additional
/// detail about which ranges failed but does not independently determine completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryResult {
    /// Retrieved historical rates.
    pub rates: Vec<Rate>,
    /// Explicit lifecycle state of the download.
    pub state: HistoryState,
    /// Time ranges `(start, end)` that failed or were missing from broker history.
    pub missing_ranges: Vec<(i64, i64)>,
}

impl HistoryResult {
    /// Construct a new [`HistoryResult`].
    pub fn new(rates: Vec<Rate>, state: HistoryState, missing_ranges: Vec<(i64, i64)>) -> Self {
        Self {
            rates,
            state,
            missing_ranges,
        }
    }

    /// Returns `true` if all requested chunks were retrieved with zero missing ranges and no cancellation.
    #[inline]
    pub fn is_complete(&self) -> bool {
        matches!(self.state, HistoryState::Complete)
    }

    /// Returns `true` if the download was cancelled before completion.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        matches!(self.state, HistoryState::Cancelled)
    }

    /// Returns `true` if the download has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_transitions_to_partial_on_data() {
        assert_eq!(
            HistoryState::Pending.transition(HistoryEvent::RatesReceived),
            HistoryState::Partial
        );
    }

    #[test]
    fn pending_stays_pending_on_all_chunks_done_with_no_data() {
        assert_eq!(
            HistoryState::Pending.transition(HistoryEvent::AllChunksDone),
            HistoryState::Pending
        );
    }

    #[test]
    fn partial_transitions_to_complete_on_all_chunks_done() {
        assert_eq!(
            HistoryState::Partial.transition(HistoryEvent::AllChunksDone),
            HistoryState::Complete
        );
    }

    #[test]
    fn partial_stays_partial_on_more_data() {
        assert_eq!(
            HistoryState::Partial.transition(HistoryEvent::RatesReceived),
            HistoryState::Partial
        );
    }

    #[test]
    fn cancelled_is_terminal_and_absorbs_all_events() {
        for event in [
            HistoryEvent::RatesReceived,
            HistoryEvent::AllChunksDone,
            HistoryEvent::Cancelled,
        ] {
            assert_eq!(
                HistoryState::Cancelled.transition(event),
                HistoryState::Cancelled,
                "Cancelled must be terminal for {:?}",
                event
            );
        }
    }

    #[test]
    fn complete_is_terminal_and_absorbs_all_events() {
        for event in [
            HistoryEvent::RatesReceived,
            HistoryEvent::AllChunksDone,
            HistoryEvent::Cancelled,
        ] {
            assert_eq!(
                HistoryState::Complete.transition(event),
                HistoryState::Complete,
                "Complete must be terminal for {:?}",
                event
            );
        }
    }

    #[test]
    fn any_non_terminal_state_transitions_to_cancelled_on_cancel() {
        for state in [HistoryState::Pending, HistoryState::Partial] {
            assert_eq!(
                state.transition(HistoryEvent::Cancelled),
                HistoryState::Cancelled,
                "{:?} must transition to Cancelled on cancel",
                state
            );
        }
        // Complete is terminal — Cancelled cannot override it.
        assert_eq!(
            HistoryState::Complete.transition(HistoryEvent::Cancelled),
            HistoryState::Complete
        );
    }

    #[test]
    fn is_complete_matches_complete_variant() {
        assert!(HistoryResult::new(vec![], HistoryState::Complete, vec![]).is_complete());
        assert!(!HistoryResult::new(vec![], HistoryState::Partial, vec![]).is_complete());
        assert!(!HistoryResult::new(vec![], HistoryState::Pending, vec![]).is_complete());
        assert!(!HistoryResult::new(vec![], HistoryState::Cancelled, vec![]).is_complete());
    }

    #[test]
    fn is_cancelled_matches_cancelled_variant() {
        assert!(HistoryResult::new(vec![], HistoryState::Cancelled, vec![]).is_cancelled());
        assert!(!HistoryResult::new(vec![], HistoryState::Complete, vec![]).is_cancelled());
        assert!(!HistoryResult::new(vec![], HistoryState::Partial, vec![]).is_cancelled());
        assert!(!HistoryResult::new(vec![], HistoryState::Pending, vec![]).is_cancelled());
    }

    #[test]
    fn priority_order_is_cancelled_then_complete_then_partial_then_pending() {
        assert_eq!(
            HistoryState::Pending.max(HistoryState::Cancelled),
            HistoryState::Cancelled
        );
        assert_eq!(
            HistoryState::Partial.max(HistoryState::Complete),
            HistoryState::Complete
        );
        assert_eq!(
            HistoryState::Complete.max(HistoryState::Partial),
            HistoryState::Complete
        );
        assert_eq!(
            HistoryState::Pending.max(HistoryState::Partial),
            HistoryState::Partial
        );
    }

    #[test]
    fn no_op_transitions_stay_in_same_state() {
        // No-op: staying in the same state on a non-applicable event
        assert_eq!(
            HistoryState::Pending.transition(HistoryEvent::AllChunksDone),
            HistoryState::Pending
        );
        // Recovery: Cancelled cannot be recovered — remains terminal
        assert_eq!(
            HistoryState::Cancelled.transition(HistoryEvent::RatesReceived),
            HistoryState::Cancelled
        );
    }
}

//! Deal-tracking position book for MetaTrader 5.
//!
//! Reconstructs position state, volumes, fill prices, and broker-side exit reasons
//! (`DealReason::StopLoss`, `DealReason::TakeProfit`, `DealReason::StopOut`, etc.) directly
//! from `OnTradeTransaction` execution deals, eliminating synthetic tick-based close inference.

use crate::types::{DealEntry, OrderType, Position, TradeEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// MT5 deal reason (`ENUM_DEAL_REASON`) indicating why a deal was executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DealReason {
    /// Deal placed manually from the terminal.
    Client,
    /// Deal placed from a mobile application.
    Mobile,
    /// Deal placed from web platform.
    Web,
    /// Deal placed from an Expert Advisor or script.
    Expert,
    /// Deal executed as a result of Stop Loss activation.
    StopLoss,
    /// Deal executed as a result of Take Profit activation.
    TakeProfit,
    /// Deal executed as a result of Stop Out (margin call / liquidation).
    StopOut,
    /// Deal executed as a result of rollover.
    Rollover,
    /// Deal executed as a result of variation margin.
    VMargin,
    /// Deal executed as a result of split.
    Split,
    /// Deal executed as a result of corporate action.
    CorporateAction,
    /// Unmapped or unknown broker deal reason code.
    Unknown(i32),
}

impl DealReason {
    /// Return raw MT5 integer deal reason code.
    pub fn as_raw(&self) -> i32 {
        match self {
            Self::Client => 0,
            Self::Mobile => 1,
            Self::Web => 2,
            Self::Expert => 3,
            Self::StopLoss => 4,
            Self::TakeProfit => 5,
            Self::StopOut => 6,
            Self::Rollover => 7,
            Self::VMargin => 8,
            Self::Split => 9,
            Self::CorporateAction => 10,
            Self::Unknown(code) => *code,
        }
    }

    /// Parse raw MT5 integer deal reason code into strongly typed [`DealReason`].
    pub fn from_raw(code: i32) -> Self {
        match code {
            0 => Self::Client,
            1 => Self::Mobile,
            2 => Self::Web,
            3 => Self::Expert,
            4 => Self::StopLoss,
            5 => Self::TakeProfit,
            6 => Self::StopOut,
            7 => Self::Rollover,
            8 => Self::VMargin,
            9 => Self::Split,
            10 => Self::CorporateAction,
            other => Self::Unknown(other),
        }
    }

    /// Returns `true` if this deal was triggered by a broker-side Stop Loss.
    pub fn is_stop_loss(&self) -> bool {
        matches!(self, Self::StopLoss)
    }

    /// Returns `true` if this deal was triggered by a broker-side Take Profit.
    pub fn is_take_profit(&self) -> bool {
        matches!(self, Self::TakeProfit)
    }

    /// Returns `true` if this deal was triggered by a broker-side Stop Out liquidation.
    pub fn is_stop_out(&self) -> bool {
        matches!(self, Self::StopOut)
    }

    /// Returns `true` if this deal was placed intentionally by client or automated API.
    pub fn is_client_or_expert(&self) -> bool {
        matches!(self, Self::Client | Self::Mobile | Self::Web | Self::Expert)
    }
}

pub use crate::types::deal::DealEntry as BookDealEntry;

/// Lifecycle state of a position within the [`PositionBook`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PositionState {
    /// Order submitted or pending fill confirmation.
    Pending,
    /// Position is confirmed open with positive active volume on the broker.
    Open,
    /// Close order has been dispatched, awaiting execution or confirmation.
    Closing,
    /// Position has been completely closed and settled.
    Closed,
    /// Position state is uncertain (e.g. close/modify timed out or lost response).
    Unknown,
}

/// Executed broker deal recorded against a position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionDeal {
    pub deal_ticket: u64,
    pub order_ticket: u64,
    pub position_ticket: u64,
    pub time: i64,
    pub entry: DealEntry,
    pub direction: OrderType,
    pub volume: f64,
    pub price: f64,
    pub reason: DealReason,
    pub comment: String,
}

/// Lightweight reference to an open or tracked position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionRef {
    /// Internal position identifier.
    pub position_id: u64,
    /// Symbol of the instrument.
    pub symbol: String,
    /// Venue-assigned ticket or order ID if known.
    pub ticket: Option<u64>,
    /// Client order ID used during position initiation.
    pub client_order_id: String,
    /// Position direction (Buy or Sell).
    pub direction: OrderType,
    /// Executed volume.
    pub volume: f64,
    /// Effective entry price.
    pub entry_price: f64,
    /// Associated magic or partition number.
    pub magic: u64,
}

impl PositionRef {
    /// Create a new lightweight position reference.
    pub fn new(
        position_id: u64,
        symbol: impl Into<String>,
        direction: OrderType,
        volume: f64,
        entry_price: f64,
    ) -> Self {
        Self {
            position_id,
            symbol: symbol.into(),
            ticket: Some(position_id),
            client_order_id: String::new(),
            direction,
            volume,
            entry_price,
            magic: 0,
        }
    }
}

impl From<&BookPosition> for PositionRef {
    fn from(pos: &BookPosition) -> Self {
        Self {
            position_id: pos.position_ticket,
            symbol: pos.symbol.clone(),
            ticket: Some(pos.position_ticket),
            client_order_id: String::new(),
            direction: pos.direction,
            volume: pos.volume,
            entry_price: pos.entry_price,
            magic: pos.magic,
        }
    }
}

/// Position tracked authoritatively by the [`PositionBook`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookPosition {
    pub position_ticket: u64,
    pub magic: u64,
    pub symbol: String,
    pub direction: OrderType,
    pub volume: f64,
    pub initial_volume: f64,
    pub entry_price: f64,
    pub entry_time: i64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub state: PositionState,
    pub deals: Vec<PositionDeal>,
    pub exit_price: Option<f64>,
    pub exit_time: Option<i64>,
    pub exit_reason: Option<DealReason>,
    pub last_deal_ticket: u64,
    pub last_updated: i64,
}

/// State transition produced when a trade event or snapshot updates the position book.
#[derive(Debug, Clone, PartialEq)]
pub enum PositionTransition {
    /// A new position was opened by an entry deal.
    Opened(BookPosition),
    /// Stop Loss or Take Profit was modified.
    Modified {
        position_ticket: u64,
        stop_loss: f64,
        take_profit: f64,
    },
    /// Position volume was reduced by an exit deal, but remaining volume remains open.
    PartiallyClosed {
        position_ticket: u64,
        closed_volume: f64,
        remaining_volume: f64,
        price: f64,
        reason: DealReason,
        deal_ticket: u64,
    },
    /// Position was completely closed by an exit deal.
    Closed {
        position_ticket: u64,
        volume: f64,
        price: f64,
        reason: DealReason,
        deal_ticket: u64,
        exit_time: i64,
    },
    /// Position lifecycle state changed (e.g. Open -> Closing, or Open -> Unknown).
    StateChanged {
        position_ticket: u64,
        old_state: PositionState,
        new_state: PositionState,
        reason: String,
    },
}

/// Authoritative position book tracking deal executions and broker state.
#[derive(Debug, Clone)]
pub struct PositionBook {
    positions: HashMap<u64, BookPosition>,
    closed_positions: HashMap<u64, BookPosition>,
    magic_filter: Option<u64>,
}

impl Default for PositionBook {
    fn default() -> Self {
        Self::new(None)
    }
}

impl PositionBook {
    /// Create a new empty position book, optionally filtering by strategy magic number.
    pub fn new(magic_filter: Option<u64>) -> Self {
        Self {
            positions: HashMap::new(),
            closed_positions: HashMap::new(),
            magic_filter,
        }
    }

    /// Apply an incoming trade transaction event and compute any position transition.
    pub fn apply_trade_event(&mut self, event: &TradeEvent) -> Option<PositionTransition> {
        // Enforce magic filter if set
        if let Some(magic) = self.magic_filter {
            if magic > 0 && event.deal_magic > 0 && event.deal_magic != magic {
                return None;
            }
        }

        // Deal events (event with positive deal ticket)
        if event.deal > 0 {
            let entry = DealEntry::from_raw(event.deal_entry);
            let reason = DealReason::from_raw(event.deal_reason);

            let deal = PositionDeal {
                deal_ticket: event.deal,
                order_ticket: event.order,
                position_ticket: event.position,
                time: event.time,
                entry,
                direction: event.order_type,
                volume: event.volume,
                price: event.price,
                reason,
                comment: event.comment.clone(),
            };

            let ticket = event.position;
            if ticket == 0 {
                return None;
            }

            match entry {
                DealEntry::In => {
                    // Position entry or add
                    if let Some(pos) = self.positions.get_mut(&ticket) {
                        // Position volume increase
                        let prev_vol = pos.volume;
                        let new_vol = prev_vol + event.volume;
                        if new_vol > 0.0 {
                            pos.entry_price =
                                (pos.entry_price * prev_vol + event.price * event.volume) / new_vol;
                        }
                        pos.volume = new_vol;
                        pos.initial_volume += event.volume;
                        pos.last_deal_ticket = event.deal;
                        pos.last_updated = event.time;
                        pos.deals.push(deal);
                        debug!(ticket, new_vol, "Position volume increased");
                        None
                    } else {
                        // Brand new position opened
                        let pos = BookPosition {
                            position_ticket: ticket,
                            magic: event.deal_magic,
                            symbol: event.symbol.clone(),
                            direction: event.order_type,
                            volume: event.volume,
                            initial_volume: event.volume,
                            entry_price: event.price,
                            entry_time: event.time,
                            stop_loss: event.sl,
                            take_profit: event.tp,
                            state: PositionState::Open,
                            deals: vec![deal],
                            exit_price: None,
                            exit_time: None,
                            exit_reason: None,
                            last_deal_ticket: event.deal,
                            last_updated: event.time,
                        };
                        self.positions.insert(ticket, pos.clone());
                        info!(
                            ticket,
                            symbol = %pos.symbol,
                            volume = pos.volume,
                            price = pos.entry_price,
                            direction = ?pos.direction,
                            "Position opened via deal"
                        );
                        Some(PositionTransition::Opened(pos))
                    }
                }
                DealEntry::Out | DealEntry::InOut | DealEntry::OutBy => {
                    // Position exit or partial close
                    if let Some(mut pos) = self.positions.remove(&ticket) {
                        let is_inout =
                            entry == DealEntry::InOut && event.volume > pos.volume + 1e-6;
                        let closed_vol = event.volume.min(pos.volume);
                        let remaining_vol = if is_inout {
                            0.0
                        } else {
                            (pos.volume - closed_vol).max(0.0)
                        };

                        pos.deals.push(deal.clone());
                        pos.last_deal_ticket = event.deal;
                        pos.last_updated = event.time;

                        if remaining_vol < 1e-6 {
                            // Fully closed
                            pos.volume = 0.0;
                            pos.state = PositionState::Closed;
                            pos.exit_price = Some(event.price);
                            pos.exit_time = Some(event.time);
                            pos.exit_reason = Some(reason);
                            self.closed_positions.insert(ticket, pos.clone());

                            info!(
                                ticket,
                                symbol = %pos.symbol,
                                exit_price = event.price,
                                reason = ?reason,
                                "Position closed via broker deal"
                            );

                            if is_inout {
                                let reversed_vol = event.volume - closed_vol;
                                let reversed_pos = BookPosition {
                                    position_ticket: ticket,
                                    magic: event.deal_magic,
                                    symbol: event.symbol.clone(),
                                    direction: event.order_type,
                                    volume: reversed_vol,
                                    initial_volume: reversed_vol,
                                    entry_price: event.price,
                                    entry_time: event.time,
                                    stop_loss: event.sl,
                                    take_profit: event.tp,
                                    state: PositionState::Open,
                                    deals: vec![deal],
                                    exit_price: None,
                                    exit_time: None,
                                    exit_reason: None,
                                    last_deal_ticket: event.deal,
                                    last_updated: event.time,
                                };
                                self.positions.insert(ticket, reversed_pos.clone());
                                info!(
                                    ticket,
                                    symbol = %reversed_pos.symbol,
                                    volume = reversed_pos.volume,
                                    price = reversed_pos.entry_price,
                                    direction = ?reversed_pos.direction,
                                    "Position reversed via InOut deal"
                                );
                            }

                            Some(PositionTransition::Closed {
                                position_ticket: ticket,
                                volume: closed_vol,
                                price: event.price,
                                reason,
                                deal_ticket: event.deal,
                                exit_time: event.time,
                            })
                        } else {
                            // Partially closed
                            pos.volume = remaining_vol;
                            self.positions.insert(ticket, pos);

                            info!(
                                ticket,
                                closed_vol,
                                remaining_vol,
                                exit_price = event.price,
                                reason = ?reason,
                                "Position partially closed via broker deal"
                            );

                            Some(PositionTransition::PartiallyClosed {
                                position_ticket: ticket,
                                closed_volume: closed_vol,
                                remaining_volume: remaining_vol,
                                price: event.price,
                                reason,
                                deal_ticket: event.deal,
                            })
                        }
                    } else {
                        // Out deal for an untracked or already-closed position
                        debug!(
                            ticket,
                            deal = event.deal,
                            price = event.price,
                            reason = ?reason,
                            "Received exit deal for untracked position"
                        );
                        None
                    }
                }
                DealEntry::Other => None,
            }
        } else if event.position > 0 {
            // Non-deal position event (e.g. SL/TP modification)
            if let Some(pos) = self.positions.get_mut(&event.position) {
                let sl_changed = (pos.stop_loss - event.sl).abs() > 1e-6 && event.sl >= 0.0;
                let tp_changed = (pos.take_profit - event.tp).abs() > 1e-6 && event.tp >= 0.0;
                if sl_changed || tp_changed {
                    pos.stop_loss = event.sl;
                    pos.take_profit = event.tp;
                    pos.last_updated = event.time;
                    return Some(PositionTransition::Modified {
                        position_ticket: event.position,
                        stop_loss: event.sl,
                        take_profit: event.tp,
                    });
                }
            }
            None
        } else {
            None
        }
    }

    /// Reconcile position book against a live snapshot of open positions from the broker.
    pub fn apply_snapshot(&mut self, broker_positions: &[Position]) -> Vec<PositionTransition> {
        let mut transitions = Vec::new();
        let mut seen_tickets = std::collections::HashSet::new();

        for bp in broker_positions {
            if let Some(magic) = self.magic_filter {
                if magic > 0 && bp.magic > 0 && bp.magic != magic {
                    continue;
                }
            }
            seen_tickets.insert(bp.ticket);

            if let Some(pos) = self.positions.get_mut(&bp.ticket) {
                // If previously Unknown or Closing, re-confirm Open if still on broker
                if pos.state == PositionState::Unknown || pos.state == PositionState::Closing {
                    let old_state = pos.state;
                    pos.state = PositionState::Open;
                    transitions.push(PositionTransition::StateChanged {
                        position_ticket: bp.ticket,
                        old_state,
                        new_state: PositionState::Open,
                        reason: "Position re-confirmed present in broker snapshot".to_string(),
                    });
                }

                // Check for SL/TP updates
                let sl_diff = (pos.stop_loss - bp.stop_loss).abs() > 1e-6;
                let tp_diff = (pos.take_profit - bp.take_profit).abs() > 1e-6;
                if sl_diff || tp_diff {
                    pos.stop_loss = bp.stop_loss;
                    pos.take_profit = bp.take_profit;
                    transitions.push(PositionTransition::Modified {
                        position_ticket: bp.ticket,
                        stop_loss: bp.stop_loss,
                        take_profit: bp.take_profit,
                    });
                }
            } else {
                // Position exists on broker but not in local book
                let pos = BookPosition {
                    position_ticket: bp.ticket,
                    magic: bp.magic,
                    symbol: bp.symbol.clone(),
                    direction: bp.position_type,
                    volume: bp.volume,
                    initial_volume: bp.volume,
                    entry_price: bp.price_open,
                    entry_time: bp.time,
                    stop_loss: bp.stop_loss,
                    take_profit: bp.take_profit,
                    state: PositionState::Open,
                    deals: Vec::new(),
                    exit_price: None,
                    exit_time: None,
                    exit_reason: None,
                    last_deal_ticket: 0,
                    last_updated: bp.time,
                };
                self.positions.insert(bp.ticket, pos.clone());
                transitions.push(PositionTransition::Opened(pos));
            }
        }

        // Detect positions locally tracked as Open that are missing from the broker snapshot
        for (ticket, pos) in self.positions.iter_mut() {
            if !seen_tickets.contains(ticket) && pos.state == PositionState::Open {
                let old_state = pos.state;
                pos.state = PositionState::Unknown;
                warn!(
                    ticket = *ticket,
                    symbol = %pos.symbol,
                    "Position missing from broker snapshot; transitioned to Unknown pending deal reconciliation"
                );
                transitions.push(PositionTransition::StateChanged {
                    position_ticket: *ticket,
                    old_state,
                    new_state: PositionState::Unknown,
                    reason: "Position missing from broker snapshot".to_string(),
                });
            }
        }

        transitions
    }

    /// Mark a position as in-flight `Closing`.
    pub fn mark_closing(&mut self, ticket: u64) -> bool {
        if let Some(pos) = self.positions.get_mut(&ticket) {
            pos.state = PositionState::Closing;
            true
        } else {
            false
        }
    }

    /// Mark a position as `Unknown` due to a lost response or transmission timeout.
    pub fn mark_unknown(&mut self, ticket: u64, reason: &str) -> bool {
        if let Some(pos) = self.positions.get_mut(&ticket) {
            let old_state = pos.state;
            pos.state = PositionState::Unknown;
            warn!(
                ticket,
                symbol = %pos.symbol,
                reason,
                ?old_state,
                "Position marked UNKNOWN; requires reconciliation before further actions"
            );
            true
        } else {
            false
        }
    }

    /// Returns `true` if any tracked position is currently in `Unknown` state.
    pub fn has_unknown(&self) -> bool {
        self.positions
            .values()
            .any(|p| p.state == PositionState::Unknown)
    }

    /// Look up a tracked open/active position by ticket.
    pub fn get(&self, ticket: u64) -> Option<&BookPosition> {
        self.positions.get(&ticket)
    }

    /// Look up a tracked open/active position mutably by ticket.
    pub fn get_mut(&mut self, ticket: u64) -> Option<&mut BookPosition> {
        self.positions.get_mut(&ticket)
    }

    /// All active (non-closed) positions currently in the book.
    pub fn open_positions(&self) -> Vec<&BookPosition> {
        self.positions
            .values()
            .filter(|p| p.state != PositionState::Closed)
            .collect()
    }

    /// All active positions for a specific symbol.
    pub fn positions_for_symbol(&self, symbol: &str) -> Vec<&BookPosition> {
        self.positions
            .values()
            .filter(|p| p.symbol == symbol && p.state != PositionState::Closed)
            .collect()
    }

    /// Recently closed positions history.
    pub fn closed_positions(&self) -> &HashMap<u64, BookPosition> {
        &self.closed_positions
    }

    /// Look up a tracked closed position by ticket.
    pub fn get_closed(&self, ticket: u64) -> Option<&BookPosition> {
        self.closed_positions.get(&ticket)
    }

    /// Calculate net volume exposure for a symbol and direction.
    pub fn net_quantity(&self, symbol: &str, direction: OrderType) -> f64 {
        self.positions
            .values()
            .filter(|p| {
                p.symbol == symbol
                    && p.direction == direction
                    && (p.state == PositionState::Open || p.state == PositionState::Closing)
            })
            .map(|p| p.volume)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_deal_event(
        deal: u64,
        position: u64,
        order_type: OrderType,
        entry: i32,
        reason: i32,
        volume: f64,
        price: f64,
    ) -> TradeEvent {
        TradeEvent {
            deal,
            order: 100,
            position,
            time: 1700000000,
            trans_type: 1, // WTX_DEAL_ADD
            order_type,
            price,
            volume,
            sl: 1.0800,
            tp: 1.0950,
            symbol: "EURUSD".to_string(),
            comment: "test".to_string(),
            deal_entry: entry,
            deal_type: order_type as i32,
            deal_magic: 12345,
            deal_position_id: position,
            deal_reason: reason,
            event_seq: 1,
        }
    }

    #[test]
    fn test_position_lifecycle_open_and_close() {
        let mut book = PositionBook::new(None);

        // Open position 1001 with 1.0 lot Buy @ 1.0850
        let open_event = test_deal_event(501, 1001, OrderType::Buy, 0, 3, 1.0, 1.0850);
        let trans = book.apply_trade_event(&open_event);
        assert!(matches!(trans, Some(PositionTransition::Opened(_))));

        let pos = book.get(1001).expect("position should exist");
        assert_eq!(pos.volume, 1.0);
        assert_eq!(pos.entry_price, 1.0850);
        assert_eq!(pos.state, PositionState::Open);
        assert_eq!(book.net_quantity("EURUSD", OrderType::Buy), 1.0);

        // Close position 1001 via Stop Loss deal @ 1.0800
        let close_event = test_deal_event(502, 1001, OrderType::Sell, 1, 4, 1.0, 1.0800);
        let trans = book.apply_trade_event(&close_event);
        assert!(matches!(
            trans,
            Some(PositionTransition::Closed {
                position_ticket: 1001,
                reason: DealReason::StopLoss,
                ..
            })
        ));

        assert!(book.get(1001).is_none());
        let closed = book.get_closed(1001).expect("closed position should exist");
        assert_eq!(closed.state, PositionState::Closed);
        assert_eq!(closed.exit_price, Some(1.0800));
        assert_eq!(closed.exit_reason, Some(DealReason::StopLoss));
        assert_eq!(book.net_quantity("EURUSD", OrderType::Buy), 0.0);
    }

    #[test]
    fn test_position_partial_close() {
        let mut book = PositionBook::new(None);

        // Open position 1002 with 2.0 lots Buy @ 1.0850
        let open_event = test_deal_event(503, 1002, OrderType::Buy, 0, 3, 2.0, 1.0850);
        book.apply_trade_event(&open_event);

        // Partially close 0.5 lots @ 1.0900 (Take Profit)
        let partial_event = test_deal_event(504, 1002, OrderType::Sell, 1, 5, 0.5, 1.0900);
        let trans = book.apply_trade_event(&partial_event);
        assert!(matches!(
            trans,
            Some(PositionTransition::PartiallyClosed {
                position_ticket: 1002,
                closed_volume,
                remaining_volume,
                reason: DealReason::TakeProfit,
                ..
            }) if (closed_volume - 0.5).abs() < 1e-6 && (remaining_volume - 1.5).abs() < 1e-6
        ));

        let pos = book.get(1002).unwrap();
        assert_eq!(pos.volume, 1.5);
        assert_eq!(book.net_quantity("EURUSD", OrderType::Buy), 1.5);
    }

    #[test]
    fn test_position_scale_in_averaging() {
        let mut book = PositionBook::new(None);

        // Open 1.0 lot @ 1.0800
        let entry1 = test_deal_event(505, 1003, OrderType::Buy, 0, 3, 1.0, 1.0800);
        book.apply_trade_event(&entry1);

        // Add 1.0 lot @ 1.0900
        let entry2 = test_deal_event(506, 1003, OrderType::Buy, 0, 3, 1.0, 1.0900);
        book.apply_trade_event(&entry2);

        let pos = book.get(1003).unwrap();
        assert_eq!(pos.volume, 2.0);
        assert!((pos.entry_price - 1.0850).abs() < 1e-6);
        assert_eq!(pos.deals.len(), 2);
    }

    #[test]
    fn test_snapshot_reconciliation() {
        let mut book = PositionBook::new(None);

        // Position 1004 is locally open
        let entry = test_deal_event(507, 1004, OrderType::Buy, 0, 3, 1.0, 1.0800);
        book.apply_trade_event(&entry);

        // Snapshot from broker: 1004 is missing, 1005 is new
        let broker_positions = vec![Position {
            ticket: 1005,
            time: 1700000100,
            position_type: OrderType::Sell,
            magic: 12345,
            volume: 0.5,
            price_open: 1.0900,
            stop_loss: 1.0950,
            take_profit: 1.0800,
            price_current: 1.0890,
            swap: 0.0,
            profit: 5.0,
            symbol: "EURUSD".to_string(),
            comment: "sync".to_string(),
        }];

        let transitions = book.apply_snapshot(&broker_positions);
        assert_eq!(transitions.len(), 2);

        // 1005 was added as Open
        assert!(book.get(1005).is_some());
        // 1004 was marked Unknown
        assert_eq!(book.get(1004).unwrap().state, PositionState::Unknown);
        assert!(book.has_unknown());
    }

    #[test]
    fn test_magic_filtering() {
        let mut book = PositionBook::new(Some(99999));

        // Event with matching magic 99999
        let mut event = test_deal_event(508, 1006, OrderType::Buy, 0, 3, 1.0, 1.0800);
        event.deal_magic = 99999;
        assert!(book.apply_trade_event(&event).is_some());

        // Event with non-matching magic 11111
        let mut foreign_event = test_deal_event(509, 1007, OrderType::Buy, 0, 3, 1.0, 1.0800);
        foreign_event.deal_magic = 11111;
        assert!(book.apply_trade_event(&foreign_event).is_none());
        assert!(book.get(1007).is_none());
    }
}

use crate::types::deal::Deal;
use crate::types::mismatch::{approx_price_eq, approx_volume_eq, AttributeMismatch, MismatchKind};
use crate::types::order_enums::{OrderState, OrderType, TradeStatus};
use crate::types::order_request::OrderRequest;
use crate::types::position::Position;
use crate::types::trade_event::{EventEffect, Fill, RejectionEvidence, TradeEvent};
use crate::types::trade_result::TradeResult;
use crate::types::wire_id::wire_id;
use crate::types::working_order::WorkingOrder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Order tracker maintaining full execution lifecycle, volume accounting, and reconciliation state.
///
/// Every field is serialisable so a `TrackedOrder` can be journaled to durable storage before
/// the corresponding request is transmitted (see [`OrderStore`](crate::OrderStore)). Fields
/// added after the first release carry `#[serde(default)]`, so state exported by older
/// versions still deserialises.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedOrder {
    /// Caller-facing idempotency key (full, untruncated).
    pub client_order_id: String,
    /// The short token that identifies this order on the wire and in MT5 comments
    /// (see [`wire_id`]). Persisted so the `client_order_id ⇄ wire token` mapping survives restarts.
    #[serde(default)]
    pub wire_id: String,
    pub symbol: String,
    pub order_type: OrderType,
    pub requested_volume: f64,
    /// Requested limit/stop price (0 for market orders).
    #[serde(default)]
    pub requested_price: f64,
    /// Requested stop loss (0 = none).
    #[serde(default)]
    pub requested_stop_loss: f64,
    /// Requested take profit (0 = none).
    #[serde(default)]
    pub requested_take_profit: f64,
    pub filled_volume: f64,
    pub remaining_volume: f64,
    pub average_price: f64,
    pub order_ticket: u64,
    pub deal_ticket: u64,
    /// Every entry-deal ticket attributed to this order (populated by deal-history reconciliation).
    #[serde(default)]
    pub deal_tickets: Vec<u64>,
    pub position_ticket: u64,
    pub state: OrderState,
    pub magic: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub error_message: Option<String>,
    /// How many consecutive reconciliation passes found this unresolved order absent everywhere.
    #[serde(default)]
    pub absent_observations: u32,
    /// Timestamp of the first of those passes.
    #[serde(default)]
    pub first_absent_at: Option<i64>,
    /// Attribute disagreements found when this order was matched to broker state.
    #[serde(default)]
    pub mismatches: Vec<AttributeMismatch>,
    /// Concrete evidence recorded when an order is determined to be Rejected.
    #[serde(default)]
    pub rejection: Option<RejectionEvidence>,
    /// Individual deal fills attributed to this order.
    #[serde(default)]
    pub fills: BTreeMap<u64, Fill>,
}

impl TrackedOrder {
    pub fn new(req: &OrderRequest, client_order_id: impl Into<String>) -> Self {
        let now = chrono::Utc::now().timestamp();
        let client_order_id = client_order_id.into();
        Self {
            wire_id: wire_id(&client_order_id),
            client_order_id,
            symbol: req.symbol.clone(),
            order_type: req.order_type,
            requested_volume: req.volume,
            requested_price: req.price,
            requested_stop_loss: req.stop_loss,
            requested_take_profit: req.take_profit,
            filled_volume: 0.0,
            remaining_volume: req.volume,
            average_price: 0.0,
            order_ticket: 0,
            deal_ticket: 0,
            deal_tickets: Vec::new(),
            position_ticket: 0,
            state: OrderState::Created,
            magic: req.magic.unwrap_or(0),
            created_at: now,
            updated_at: now,
            error_message: None,
            absent_observations: 0,
            first_absent_at: None,
            mismatches: Vec::new(),
            rejection: None,
            fills: BTreeMap::new(),
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
        if res.deal > 0 && !self.deal_tickets.contains(&res.deal) {
            self.deal_tickets.push(res.deal);
        }
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

    /// Update order state from an asynchronous broker `TradeEvent` (protocol v5+ push model).
    pub fn update_from_trade_event(&mut self, ev: &TradeEvent) -> EventEffect {
        if matches!(
            self.state,
            OrderState::Filled | OrderState::Cancelled | OrderState::Rejected
        ) {
            return EventEffect::TerminalStateIgnored;
        }

        self.updated_at = chrono::Utc::now().timestamp();
        if ev.order > 0 && self.order_ticket == 0 {
            self.order_ticket = ev.order;
        }
        if ev.deal_position_id > 0 && self.position_ticket == 0 {
            self.position_ticket = ev.deal_position_id;
        } else if ev.position > 0 && self.position_ticket == 0 {
            self.position_ticket = ev.position;
        }

        if ev.deal > 0 {
            if self.fills.contains_key(&ev.deal) {
                return EventEffect::Duplicate;
            }
            if ev.deal_entry == 1 {
                // Exit deal (OUT)
                return EventEffect::ExitDealIgnored;
            }
            let fill = Fill::from(ev);
            self.fills.insert(ev.deal, fill);
            if !self.deal_tickets.contains(&ev.deal) {
                self.deal_tickets.push(ev.deal);
            }
            self.deal_ticket = ev.deal;

            let total_filled: f64 = self.fills.values().map(|f| f.volume).sum();
            let total_value: f64 = self.fills.values().map(|f| f.volume * f.price).sum();
            self.filled_volume = total_filled.min(self.requested_volume);
            self.remaining_volume = (self.requested_volume - total_filled).max(0.0);
            if total_filled > 0.0 {
                self.average_price = total_value / total_filled;
            }
            if self.remaining_volume < 1e-6 {
                self.state = OrderState::Filled;
            } else {
                self.state = OrderState::PartiallyFilled;
            }
            self.reset_absence();
            return EventEffect::Applied;
        } else if self.state == OrderState::Submitting || self.state == OrderState::Created {
            self.state = OrderState::Accepted;
            self.reset_absence();
            return EventEffect::Applied;
        }

        EventEffect::Ignored
    }

    /// Mark order status as Unknown when network transmission is ambiguous (e.g. pipe broken during response).
    pub fn mark_unknown(&mut self, reason: impl Into<String>) {
        self.state = OrderState::Unknown;
        self.error_message = Some(reason.into());
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Returns `true` while the outcome of the request is still undetermined
    /// (`Unknown`, or `Submitting` — which after a crash is equivalent to `Unknown`).
    pub fn is_unresolved(&self) -> bool {
        matches!(self.state, OrderState::Unknown | OrderState::Submitting)
    }

    /// Clear the "absent from broker" observation counters (the order was seen, or was resolved).
    pub fn reset_absence(&mut self) {
        self.absent_observations = 0;
        self.first_absent_at = None;
    }

    fn check_symbol(&self, actual: &str, out: &mut Vec<AttributeMismatch>) {
        if !self.symbol.eq_ignore_ascii_case(actual) {
            out.push(AttributeMismatch {
                kind: MismatchKind::Symbol,
                expected: self.symbol.clone(),
                actual: actual.to_string(),
            });
        }
    }

    fn check_direction(&self, actual_is_buy: bool, out: &mut Vec<AttributeMismatch>) {
        if self.order_type.is_buy() != actual_is_buy {
            let name = |b: bool| if b { "buy" } else { "sell" }.to_string();
            out.push(AttributeMismatch {
                kind: MismatchKind::Direction,
                expected: name(self.order_type.is_buy()),
                actual: name(actual_is_buy),
            });
        }
    }

    fn check_magic(&self, actual: u64, out: &mut Vec<AttributeMismatch>) {
        // A tracked order created without an explicit magic (0) makes no ownership claim.
        if self.magic != 0 && self.magic != actual {
            out.push(AttributeMismatch {
                kind: MismatchKind::Magic,
                expected: self.magic.to_string(),
                actual: actual.to_string(),
            });
        }
    }

    fn check_volume(&self, expected: f64, actual: f64, out: &mut Vec<AttributeMismatch>) {
        if !approx_volume_eq(expected, actual) {
            out.push(AttributeMismatch {
                kind: MismatchKind::Volume,
                expected: expected.to_string(),
                actual: actual.to_string(),
            });
        }
    }

    fn check_protection(&self, sl: f64, tp: f64, out: &mut Vec<AttributeMismatch>) {
        if !approx_price_eq(self.requested_stop_loss, sl) {
            out.push(AttributeMismatch {
                kind: MismatchKind::StopLoss,
                expected: self.requested_stop_loss.to_string(),
                actual: sl.to_string(),
            });
        }
        if !approx_price_eq(self.requested_take_profit, tp) {
            out.push(AttributeMismatch {
                kind: MismatchKind::TakeProfit,
                expected: self.requested_take_profit.to_string(),
                actual: tp.to_string(),
            });
        }
    }

    fn compare_working_order(
        &self,
        ord: &WorkingOrder,
        check_volume: bool,
    ) -> Vec<AttributeMismatch> {
        let mut mm = Vec::new();
        self.check_symbol(&ord.symbol, &mut mm);
        if self.order_type != ord.order_type {
            if self.order_type.is_buy() != ord.order_type.is_buy() {
                self.check_direction(ord.order_type.is_buy(), &mut mm);
            }
            mm.push(AttributeMismatch {
                kind: MismatchKind::OrderType,
                expected: format!("{:?}", self.order_type),
                actual: format!("{:?}", ord.order_type),
            });
        }
        if check_volume {
            self.check_volume(self.requested_volume, ord.volume_initial, &mut mm);
        }
        if !approx_price_eq(self.requested_price, ord.price_open) {
            mm.push(AttributeMismatch {
                kind: MismatchKind::Price,
                expected: self.requested_price.to_string(),
                actual: ord.price_open.to_string(),
            });
        }
        self.check_protection(ord.stop_loss, ord.take_profit, &mut mm);
        self.check_magic(ord.magic, &mut mm);
        mm
    }

    fn finish_reconcile(&mut self, ok_state: OrderState, mm: Vec<AttributeMismatch>) {
        self.state = if mm.is_empty() {
            ok_state
        } else {
            OrderState::Mismatched
        };
        self.mismatches = mm;
        self.reset_absence();
        self.error_message = None;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Reconcile against a live position, **comparing** symbol, direction, volume, SL, TP and
    /// magic with the original request.
    ///
    /// The order becomes `Reconciled` only if every attribute agrees; otherwise it becomes
    /// `Mismatched` and the returned list (also stored in [`mismatches`](Self::mismatches))
    /// says exactly what differs. Fill data is taken from the position either way, because
    /// the exposure is real regardless.
    pub fn reconcile_with_position(&mut self, pos: &Position) -> Vec<AttributeMismatch> {
        let mut mm = Vec::new();
        self.check_symbol(&pos.symbol, &mut mm);
        self.check_direction(pos.is_buy(), &mut mm);
        self.check_volume(self.requested_volume, pos.volume, &mut mm);
        self.check_protection(pos.stop_loss, pos.take_profit, &mut mm);
        self.check_magic(pos.magic, &mut mm);

        self.position_ticket = pos.ticket;
        self.filled_volume = pos.volume;
        self.remaining_volume = (self.requested_volume - pos.volume).max(0.0);
        self.average_price = pos.price_open;
        self.finish_reconcile(OrderState::Reconciled, mm.clone());
        mm
    }

    /// Reconcile against a live pending order, comparing type, symbol, volume, price, SL, TP and magic.
    pub fn reconcile_with_working_order(&mut self, ord: &WorkingOrder) -> Vec<AttributeMismatch> {
        let mm = self.compare_working_order(ord, true);
        self.order_ticket = ord.ticket;
        self.filled_volume = ord.volume_initial - ord.volume_current;
        self.remaining_volume = ord.volume_current;
        self.finish_reconcile(OrderState::Accepted, mm.clone());
        mm
    }

    /// Reconstruct fills from deal history — the authoritative record of what executed.
    ///
    /// `entry_deals` must be the entry deals attributed to this order. Filled volume becomes
    /// their sum and the average price their volume-weighted mean, so an order filled as
    /// 0.4 + 0.6 lots is reconstructed as exactly 1.0 lot even if the position has since been
    /// closed or (netting accounts) merged with other exposure.
    ///
    /// * `position` — the live position these deals belong to, if any; used to verify SL/TP.
    /// * `working` — a still-working remainder of this order, if any; the order is then
    ///   `PartiallyFilled` rather than volume-mismatched.
    pub fn reconcile_with_deals(
        &mut self,
        entry_deals: &[&Deal],
        position: Option<&Position>,
        working: Option<&WorkingOrder>,
    ) -> Vec<AttributeMismatch> {
        let mut mm = Vec::new();
        let filled: f64 = entry_deals.iter().map(|d| d.volume).sum();
        let notional: f64 = entry_deals.iter().map(|d| d.volume * d.price).sum();

        let mut tickets: Vec<u64> = entry_deals.iter().map(|d| d.ticket).collect();
        tickets.sort_unstable();
        tickets.dedup();
        self.deal_ticket = tickets.last().copied().unwrap_or(self.deal_ticket);
        self.deal_tickets = tickets;
        self.filled_volume = filled;
        self.average_price = if filled > 0.0 { notional / filled } else { 0.0 };
        if let Some(d) = entry_deals.iter().find(|d| d.position_id > 0) {
            self.position_ticket = d.position_id;
        }
        if self.order_ticket == 0 {
            if let Some(d) = entry_deals.iter().find(|d| d.order > 0) {
                self.order_ticket = d.order;
            }
        }

        // Every deal must agree on symbol / direction / magic with the request.
        for d in entry_deals {
            self.check_symbol(&d.symbol, &mut mm);
            self.check_direction(d.direction.is_buy(), &mut mm);
            self.check_magic(d.magic, &mut mm);
        }
        mm.dedup_by(|a, b| a.kind == b.kind && a.actual == b.actual);

        let ok_state;
        if let Some(w) = working {
            mm.extend(self.compare_working_order(w, false));
            self.order_ticket = w.ticket;
            self.remaining_volume = w.volume_current;
            ok_state = OrderState::PartiallyFilled;
        } else {
            self.remaining_volume = (self.requested_volume - filled).max(0.0);
            self.check_volume(self.requested_volume, filled, &mut mm);
            if let Some(pos) = position {
                self.check_protection(pos.stop_loss, pos.take_profit, &mut mm);
            }
            ok_state = OrderState::Reconciled;
        }
        self.finish_reconcile(ok_state, mm.clone());
        mm
    }

    /// Returns `true` if this order has reached a final state (Filled, Cancelled, Rejected).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            OrderState::Filled | OrderState::Cancelled | OrderState::Rejected
        )
    }

    /// Returns `true` if this order is still open or working (anything that is not terminal).
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

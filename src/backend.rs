//! Abstraction over "something that can execute and query trades".
//!
//! [`OrderManager`](crate::OrderManager) is written against [`TradingBackend`] rather than the
//! concrete [`Mt5Client`], which is what makes failure-injection testing possible: a test can
//! substitute a simulated broker that drops responses after execution, delays position
//! visibility, restarts the "EA", and so on — none of which can be triggered on demand
//! against a real terminal.
//!
//! [`Mt5Client`] implements the trait by delegating to its inherent methods, so production
//! code is unaffected: `manager.submit_order(&client, req)` keeps working unchanged.

use crate::client::Mt5Client;
use crate::error::{Mt5Error, Result};
use crate::types::{Deal, OrderRequest, Position, TradeResult, WorkingOrder};
use std::sync::Arc;

/// Trading operations required by [`OrderManager`](crate::OrderManager).
pub trait TradingBackend {
    /// Submit an order. Must return [`Mt5Error::UnknownExecutionState`] when the request may
    /// have reached the broker but no response was received, and
    /// [`Mt5Error::TransmissionFailed`] only when it certainly did not.
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult>;

    /// Close a position, verifying ownership by magic number.
    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult>;

    /// Modify SL/TP of a position or pending order, verifying ownership by magic number.
    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult>;

    /// Open positions, optionally filtered by magic number and/or symbol.
    fn positions_filtered(&self, magic: Option<u64>, symbol: Option<&str>)
        -> Result<Vec<Position>>;

    /// Working pending orders, optionally filtered by magic number and/or symbol.
    fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>>;

    /// Completed trade deals with `from <= time <= to` (UTC seconds).
    ///
    /// Backends without trade-history support keep this default; reconciliation then treats
    /// deal history as unavailable and will not auto-reject absent orders under the default
    /// safety policy.
    fn deals_filtered(
        &self,
        _from: i64,
        _to: i64,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        Err(Mt5Error::UnsupportedFeature("DealsGet"))
    }
}

impl TradingBackend for Mt5Client {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        Mt5Client::order_send(self, req)
    }

    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        Mt5Client::order_close_with_magic(self, ticket, magic)
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        Mt5Client::order_modify_with_magic(self, ticket, magic, stop_loss, take_profit)
    }

    fn positions_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        Mt5Client::positions_filtered(self, magic, symbol)
    }

    fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        Mt5Client::pending_orders_filtered(self, magic, symbol)
    }

    fn deals_filtered(
        &self,
        from: i64,
        to: i64,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        Mt5Client::deals_filtered(self, from, to, magic, symbol)
    }
}

// Ergonomics: keep `manager.submit_order(&arc_client, ..)` and `&&client` working now that the
// manager is generic over the backend (deref coercion does not apply to generic parameters).
impl<T: TradingBackend + ?Sized> TradingBackend for Arc<T> {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        (**self).order_send(req)
    }
    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        (**self).order_close_with_magic(ticket, magic)
    }
    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        (**self).order_modify_with_magic(ticket, magic, stop_loss, take_profit)
    }
    fn positions_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        (**self).positions_filtered(magic, symbol)
    }
    fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        (**self).pending_orders_filtered(magic, symbol)
    }
    fn deals_filtered(
        &self,
        from: i64,
        to: i64,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        (**self).deals_filtered(from, to, magic, symbol)
    }
}

impl<T: TradingBackend + ?Sized> TradingBackend for &T {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        (**self).order_send(req)
    }
    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        (**self).order_close_with_magic(ticket, magic)
    }
    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        (**self).order_modify_with_magic(ticket, magic, stop_loss, take_profit)
    }
    fn positions_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        (**self).positions_filtered(magic, symbol)
    }
    fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        (**self).pending_orders_filtered(magic, symbol)
    }
    fn deals_filtered(
        &self,
        from: i64,
        to: i64,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        (**self).deals_filtered(from, to, magic, symbol)
    }
}

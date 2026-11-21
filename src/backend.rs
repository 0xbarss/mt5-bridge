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
use crate::types::{
    AccountInfo, Deal, HistoryResult, Level, OrderRequest, Position, Rate, Timeframe, TradeResult,
    WorkingOrder,
};
use std::sync::Arc;

/// Trading operations required by [`OrderManager`](crate::OrderManager).
pub trait TradingBackend {
    /// Submit an order. Must return [`Mt5Error::UnknownExecutionState`] when the request may
    /// have reached the broker but no response was received, and
    /// [`Mt5Error::TransmissionFailed`] only when it certainly did not.
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult>;

    /// Close a position, verifying ownership by magic number.
    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult>;

    /// Close an open market position with custom deviation and volume limits.
    fn order_close_with_deviation(
        &self,
        ticket: u64,
        magic: u64,
        _deviation: u32,
        _volume: f64,
    ) -> Result<TradeResult> {
        self.order_close_with_magic(ticket, magic)
    }

    /// Close an open market position by ticket, verifying strategy magic number ownership.
    ///
    /// This is an ergonomic alias for [`order_close_with_magic`](Self::order_close_with_magic).
    fn close_position_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        self.order_close_with_magic(ticket, magic)
    }

    /// Modify SL/TP of a position or pending order, verifying ownership by magic number.
    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: Level,
        take_profit: Level,
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

    /// Account balance, equity, and margin information.
    fn account_info(&self) -> Result<AccountInfo> {
        Err(Mt5Error::UnsupportedFeature("AccountInfo"))
    }

    /// Fetch historical rates for a symbol and timeframe.
    fn copy_rates(
        &self,
        _symbol: &str,
        _timeframe: Timeframe,
        _from: i64,
        _to: i64,
    ) -> Result<Vec<Rate>> {
        Err(Mt5Error::UnsupportedFeature("CopyRates"))
    }

    /// Chunked historical-rate download with cooperative cancellation.
    ///
    /// Implementations should check `cancel` between chunks and return a partial
    /// [`HistoryResult`] with `cancelled: true` when set. The default delegates to
    /// [`copy_rates`](Self::copy_rates) and ignores the cancel token.
    fn copy_rates_chunked(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<HistoryResult> {
        let _ = cancel;
        let rates = self.copy_rates(symbol, timeframe, from, to)?;
        Ok(HistoryResult {
            rates,
            complete: true,
            missing_ranges: vec![],
            cancelled: false,
        })
    }

    /// Check backend responsiveness via heartbeat ping.
    fn ping(&self) -> Result<()> {
        self.account_info().map(|_| ())
    }

    /// Subscribe to real-time tick quotes.
    fn subscribe_ticks(&self, _symbol: &str) -> Result<()> {
        Ok(())
    }

    /// Subscribe to real-time broker trade events.
    fn subscribe_trade(&self) -> Result<()> {
        Ok(())
    }

    /// Subscribe to depth-of-market book updates.
    fn subscribe_book(&self, _symbol: &str) -> Result<()> {
        Ok(())
    }

    /// Subscribe to closed-bar notifications.
    fn subscribe_bars(&self, _symbol: &str, _timeframe: Timeframe) -> Result<()> {
        Ok(())
    }

    /// Subscribe to connection disconnect notifications if supported by backend.
    #[cfg(feature = "async")]
    fn subscribe_disconnect(&self) -> Option<tokio::sync::broadcast::Receiver<()>> {
        None
    }
}

impl TradingBackend for Mt5Client {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        Mt5Client::order_send(self, req)
    }

    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        Mt5Client::order_close_with_magic(self, ticket, magic)
    }

    fn order_close_with_deviation(
        &self,
        ticket: u64,
        magic: u64,
        deviation: u32,
        volume: f64,
    ) -> Result<TradeResult> {
        Mt5Client::order_close_with_deviation(self, ticket, magic, deviation, volume)
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: Level,
        take_profit: Level,
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

    fn account_info(&self) -> Result<AccountInfo> {
        Mt5Client::account_info(self)
    }

    fn copy_rates(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Rate>> {
        Mt5Client::copy_rates(self, symbol, timeframe, from, to)
    }

    fn copy_rates_chunked(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<HistoryResult> {
        Mt5Client::copy_rates_chunked_cancellable(
            self,
            symbol,
            timeframe,
            from,
            to,
            2000,
            Some(cancel),
        )
    }

    fn ping(&self) -> Result<()> {
        Mt5Client::ping(self).map(|_| ())
    }

    fn subscribe_ticks(&self, symbol: &str) -> Result<()> {
        #[cfg(feature = "async")]
        {
            Mt5Client::subscribe_ticks(self, symbol).map(|_| ())
        }
        #[cfg(not(feature = "async"))]
        {
            let _ = symbol;
            Ok(())
        }
    }

    fn subscribe_trade(&self) -> Result<()> {
        #[cfg(feature = "async")]
        {
            Mt5Client::subscribe_trade(self).map(|_| ())
        }
        #[cfg(not(feature = "async"))]
        {
            Ok(())
        }
    }

    fn subscribe_book(&self, symbol: &str) -> Result<()> {
        #[cfg(feature = "async")]
        {
            Mt5Client::subscribe_book(self, symbol).map(|_| ())
        }
        #[cfg(not(feature = "async"))]
        {
            let _ = symbol;
            Ok(())
        }
    }

    fn subscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<()> {
        #[cfg(feature = "async")]
        {
            Mt5Client::subscribe_bars(self, symbol, timeframe).map(|_| ())
        }
        #[cfg(not(feature = "async"))]
        {
            let _ = (symbol, timeframe);
            Ok(())
        }
    }

    #[cfg(feature = "async")]
    fn subscribe_disconnect(&self) -> Option<tokio::sync::broadcast::Receiver<()>> {
        Some(self.subscribe_disconnect())
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
    fn order_close_with_deviation(
        &self,
        ticket: u64,
        magic: u64,
        deviation: u32,
        volume: f64,
    ) -> Result<TradeResult> {
        (**self).order_close_with_deviation(ticket, magic, deviation, volume)
    }
    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: Level,
        take_profit: Level,
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
    fn account_info(&self) -> Result<AccountInfo> {
        (**self).account_info()
    }
    fn copy_rates(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Rate>> {
        (**self).copy_rates(symbol, timeframe, from, to)
    }

    fn copy_rates_chunked(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<HistoryResult> {
        (**self).copy_rates_chunked(symbol, timeframe, from, to, cancel)
    }

    fn ping(&self) -> Result<()> {
        (**self).ping()
    }
    fn subscribe_ticks(&self, symbol: &str) -> Result<()> {
        (**self).subscribe_ticks(symbol)
    }
    fn subscribe_trade(&self) -> Result<()> {
        (**self).subscribe_trade()
    }
    fn subscribe_book(&self, symbol: &str) -> Result<()> {
        (**self).subscribe_book(symbol)
    }
    fn subscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<()> {
        (**self).subscribe_bars(symbol, timeframe)
    }
    #[cfg(feature = "async")]
    fn subscribe_disconnect(&self) -> Option<tokio::sync::broadcast::Receiver<()>> {
        (**self).subscribe_disconnect()
    }
}

impl<T: TradingBackend + ?Sized> TradingBackend for &T {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        (**self).order_send(req)
    }
    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        (**self).order_close_with_magic(ticket, magic)
    }
    fn order_close_with_deviation(
        &self,
        ticket: u64,
        magic: u64,
        deviation: u32,
        volume: f64,
    ) -> Result<TradeResult> {
        (**self).order_close_with_deviation(ticket, magic, deviation, volume)
    }
    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: Level,
        take_profit: Level,
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
    fn account_info(&self) -> Result<AccountInfo> {
        (**self).account_info()
    }
    fn copy_rates(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Rate>> {
        (**self).copy_rates(symbol, timeframe, from, to)
    }

    fn copy_rates_chunked(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<HistoryResult> {
        (**self).copy_rates_chunked(symbol, timeframe, from, to, cancel)
    }

    fn ping(&self) -> Result<()> {
        (**self).ping()
    }
    fn subscribe_ticks(&self, symbol: &str) -> Result<()> {
        (**self).subscribe_ticks(symbol)
    }
    fn subscribe_trade(&self) -> Result<()> {
        (**self).subscribe_trade()
    }
    fn subscribe_book(&self, symbol: &str) -> Result<()> {
        (**self).subscribe_book(symbol)
    }
    fn subscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<()> {
        (**self).subscribe_bars(symbol, timeframe)
    }
    #[cfg(feature = "async")]
    fn subscribe_disconnect(&self) -> Option<tokio::sync::broadcast::Receiver<()>> {
        (**self).subscribe_disconnect()
    }
}

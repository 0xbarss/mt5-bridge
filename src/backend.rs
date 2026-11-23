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
    AccountInfo, Deal, DealEntry, HistoryResult, HistoryState, Level, OrderRequest, OrderType,
    Position, Rate, Timeframe, TradeResult, WorkingOrder,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
        Ok(HistoryResult::new(rates, HistoryState::Complete, vec![]))
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

// =============================================================================
// Dry-run backend — executes against an in-memory order book with synthetic fills,
// so strategies can be tested without a broker and no real orders are ever placed.
// =============================================================================

/// Fill model used by [`DryRunBackend`] to decide when an order fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FillModel {
    /// Fill immediately on submission at the configured fill price.
    #[default]
    Immediate,
    /// Fill at the next tick/bar that moves the price through the stop level (pending orders
    /// only; market orders fill immediately).
    NextTick,
    /// Fill when a bar closes at or beyond the stop level (pending orders only).
    BarClose,
}

/// Configuration for [`DryRunBackend`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DryRunConfig {
    /// How synthetic fills are triggered.
    pub fill_model: FillModel,
    /// Starting account balance (deposit currency).
    pub initial_balance: f64,
    /// Price slippage applied to fills (price units), default 0.
    pub slippage_bps: f64,
    /// Price used to fill market orders when `fill_model` is `Immediate`; defaults to `1.0`.
    pub default_fill_price: f64,
}

impl Default for DryRunConfig {
    fn default() -> Self {
        Self {
            fill_model: FillModel::Immediate,
            initial_balance: 10_000.0,
            slippage_bps: 0.0,
            default_fill_price: 1.0,
        }
    }
}

/// A synthetic position held by [`DryRunBackend`].
#[derive(Debug, Clone)]
struct DryPosition {
    ticket: u64,
    symbol: String,
    position_type: OrderType,
    volume: f64,
    price_open: f64,
    stop_loss: f64,
    take_profit: f64,
    price_current: f64,
    magic: u64,
}

/// A synthetic working (pending) order held by [`DryRunBackend`].
#[derive(Debug, Clone)]
struct DryWorkingOrder {
    ticket: u64,
    symbol: String,
    order_type: OrderType,
    volume_initial: f64,
    volume_current: f64,
    price_open: f64,
    stop_loss: f64,
    take_profit: f64,
    price_current: f64,
    magic: u64,
}

/// A synthetic completed deal.
#[derive(Debug, Clone)]
struct DryDeal {
    ticket: u64,
    order: u64,
    position_id: u64,
    time: i64,
    direction: OrderType,
    entry: DealEntry,
    magic: u64,
    volume: f64,
    price: f64,
    symbol: String,
}

/// Synthetic backend: in-memory positions, working orders, and deals.
///
/// Submits are journaled locally, never transmitted to MT5. Market orders fill immediately
/// (or per `fill_model`) against a synthetic price stream you drive via [`tick`](DryRunBackend::tick) /
/// [`bar`](DryRunBackend::bar) updates. Never places real orders.
#[derive(Debug, Default)]
pub struct DryRunBackend {
    config: DryRunConfig,
    state: Mutex<DryRunState>,
}

#[derive(Debug, Default)]
struct DryRunState {
    next_ticket: u64,
    balance: f64,
    equity: f64,
    positions: Vec<DryPosition>,
    pending: Vec<DryWorkingOrder>,
    deals: Vec<DryDeal>,
    /// Latest synthetic price per symbol (bid for buys / ask for sells).
    prices: HashMap<String, (f64 /*bid*/, f64 /*ask*/)>,
}

impl DryRunBackend {
    /// Create a new dry-run backend with the given config.
    pub fn new(config: DryRunConfig) -> Self {
        let state = DryRunState {
            next_ticket: 1,
            balance: config.initial_balance,
            equity: config.initial_balance,
            ..Default::default()
        };
        Self {
            config,
            state: Mutex::new(state),
        }
    }

    /// Inject a synthetic tick (bid/ask) for `symbol`, which may trigger pending-order fills
    /// under the `NextTick` fill model.
    pub fn tick(&self, symbol: &str, bid: f64, ask: f64) {
        let mut g = self.state.lock().unwrap();
        g.prices.insert(symbol.to_string(), (bid, ask));
        let pending: Vec<DryWorkingOrder> = std::mem::take(&mut g.pending);
        for order in pending {
            let mut filled = false;
            if order.order_type.is_buy() {
                if bid <= order.price_open {
                    filled = true;
                }
            } else if ask >= order.price_open {
                filled = true;
            }
            if filled {
                self.fill_pending(&mut g, order, bid, ask);
            } else {
                g.pending.push(order);
            }
        }
    }

    /// Inject a synthetic closed bar for `symbol`; fills pending orders whose stop level was
    /// touched at bar close under the `BarClose` fill model.
    pub fn bar(&self, symbol: &str, close: f64) {
        let mut g = self.state.lock().unwrap();
        g.prices.entry(symbol.to_string()).or_insert((close, close));
        let pending: Vec<DryWorkingOrder> = std::mem::take(&mut g.pending);
        for order in pending {
            let mut filled = false;
            if order.order_type.is_buy() {
                if close <= order.price_open {
                    filled = true;
                }
            } else if close >= order.price_open {
                filled = true;
            }
            if filled {
                self.fill_pending(&mut g, order, close, close);
            } else {
                g.pending.push(order);
            }
        }
    }

    fn fill_pending(&self, g: &mut DryRunState, order: DryWorkingOrder, price: f64, _ask: f64) {
        g.next_ticket += 1;
        let position_ticket = g.next_ticket;
        g.next_ticket += 1;
        let deal_ticket = g.next_ticket;
        g.next_ticket += 1;

        let slippage = self.config.slippage_bps / 10_000.0;
        let fill_price = if order.order_type.is_buy() {
            price * (1.0 + slippage)
        } else {
            price * (1.0 - slippage)
        };

        g.positions.push(DryPosition {
            ticket: position_ticket,
            symbol: order.symbol.clone(),
            position_type: order.order_type,
            volume: order.volume_initial,
            price_open: fill_price,
            stop_loss: order.stop_loss,
            take_profit: order.take_profit,
            price_current: fill_price,
            magic: order.magic,
        });
        g.deals.push(DryDeal {
            ticket: deal_ticket,
            order: order.ticket,
            position_id: position_ticket,
            time: Utc::now().timestamp(),
            direction: order.order_type,
            entry: DealEntry::In,
            magic: order.magic,
            volume: order.volume_initial,
            price: fill_price,
            symbol: order.symbol.clone(),
        });
        g.balance += if order.order_type.is_buy() {
            -order.volume_initial * fill_price
        } else {
            order.volume_initial * fill_price
        };
        g.equity = g.balance;
    }

    /// Latest synthetic price for `symbol`, if a tick or bar has been injected.
    pub fn price(&self, symbol: &str) -> Option<(f64, f64)> {
        self.state.lock().unwrap().prices.get(symbol).copied()
    }

    /// Synthetic account snapshot.
    pub fn account(&self) -> AccountInfo {
        let g = self.state.lock().unwrap();
        AccountInfo {
            balance: g.balance,
            equity: g.equity,
            margin: 0.0,
            free_margin: g.balance,
            leverage: 0.0,
            currency: String::new(),
        }
    }

    /// All synthetic positions.
    pub fn positions(&self) -> Vec<Position> {
        let g = self.state.lock().unwrap();
        g.positions
            .iter()
            .map(|p| Position {
                ticket: p.ticket,
                time: Utc::now().timestamp(),
                position_type: p.position_type,
                magic: p.magic,
                volume: p.volume,
                price_open: p.price_open,
                stop_loss: p.stop_loss,
                take_profit: p.take_profit,
                price_current: p.price_current,
                profit: 0.0,
                swap: 0.0,
                symbol: p.symbol.clone(),
                comment: String::new(),
            })
            .collect()
    }
}

impl TradingBackend for DryRunBackend {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        match req.order_type {
            OrderType::Buy | OrderType::Sell => {
                // Market order: fill immediately at the configured price.
                let price = match self.config.fill_model {
                    FillModel::Immediate => self.config.default_fill_price,
                    FillModel::NextTick | FillModel::BarClose => {
                        // No tick/bar stream yet: leave as accepted (working) until a tick
                        // drives the price through the level. Market orders have no stop level
                        // to wait on, so fill at the last known price if available, else default.
                        let (bid, ask) = g.prices.get(&req.symbol).copied().unwrap_or((
                            self.config.default_fill_price,
                            self.config.default_fill_price,
                        ));
                        if req.order_type.is_buy() {
                            bid
                        } else {
                            ask
                        }
                    }
                };
                let slippage = self.config.slippage_bps / 10_000.0;
                let fill_price = if req.order_type.is_buy() {
                    price * (1.0 + slippage)
                } else {
                    price * (1.0 - slippage)
                };
                let position_ticket = g.next_ticket;
                g.next_ticket += 1;
                let deal_ticket = g.next_ticket;
                g.next_ticket += 1;

                g.positions.push(DryPosition {
                    ticket: position_ticket,
                    symbol: req.symbol.clone(),
                    position_type: req.order_type,
                    volume: req.volume,
                    price_open: fill_price,
                    stop_loss: req.stop_loss,
                    take_profit: req.take_profit,
                    price_current: fill_price,
                    magic: req.magic.unwrap_or(0),
                });
                g.deals.push(DryDeal {
                    ticket: deal_ticket,
                    order: position_ticket,
                    position_id: position_ticket,
                    time: Utc::now().timestamp(),
                    direction: req.order_type,
                    entry: DealEntry::In,
                    magic: req.magic.unwrap_or(0),
                    volume: req.volume,
                    price: fill_price,
                    symbol: req.symbol.clone(),
                });
                g.balance -= req.volume * fill_price;
                g.equity = g.balance;

                Ok(TradeResult {
                    retcode: 10009,
                    deal: deal_ticket,
                    order: position_ticket,
                    position: position_ticket,
                    volume: req.volume,
                    price: fill_price,
                })
            }
            OrderType::BuyLimit
            | OrderType::SellLimit
            | OrderType::BuyStop
            | OrderType::SellStop => {
                // Pending order: working until a tick/bar drives the price through the level.
                let price_current = g.prices.get(&req.symbol).map(|(_, a)| *a).unwrap_or(0.0);
                let working_ticket = g.next_ticket;
                g.next_ticket += 1;
                g.pending.push(DryWorkingOrder {
                    ticket: working_ticket,
                    symbol: req.symbol.clone(),
                    order_type: req.order_type,
                    volume_initial: req.volume,
                    volume_current: req.volume,
                    price_open: req.price,
                    stop_loss: req.stop_loss,
                    take_profit: req.take_profit,
                    price_current,
                    magic: req.magic.unwrap_or(0),
                });
                Ok(TradeResult {
                    retcode: 10008,
                    deal: 0,
                    order: working_ticket,
                    position: 0,
                    volume: 0.0,
                    price: req.price,
                })
            }
        }
    }

    fn order_close_with_magic(&self, ticket: u64, _magic: u64) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        let Some(i) = g.positions.iter().position(|p| p.ticket == ticket) else {
            return Err(Mt5Error::Other(format!("position {ticket} not found")));
        };
        let p = g.positions.remove(i);
        let deal_ticket = g.next_ticket;
        g.next_ticket += 1;
        g.deals.push(DryDeal {
            ticket: deal_ticket,
            order: 0,
            position_id: ticket,
            time: Utc::now().timestamp(),
            direction: if p.position_type.is_buy() {
                OrderType::Sell
            } else {
                OrderType::Buy
            },
            entry: DealEntry::Out,
            magic: p.magic,
            volume: p.volume,
            price: p.price_current,
            symbol: p.symbol.clone(),
        });
        g.balance += p.volume * p.price_current;
        g.equity = g.balance;
        Ok(TradeResult {
            retcode: 10009,
            deal: deal_ticket,
            order: 0,
            position: ticket,
            volume: p.volume,
            price: p.price_current,
        })
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        _magic: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        let mut price_open = 0.0f64;
        let modified = if let Some(pos) = g.positions.iter_mut().find(|p| p.ticket == ticket) {
            price_open = pos.price_open;
            match stop_loss {
                Level::Set(v) => pos.stop_loss = v,
                Level::Clear => pos.stop_loss = 0.0,
                Level::Keep => {}
            }
            match take_profit {
                Level::Set(v) => pos.take_profit = v,
                Level::Clear => pos.take_profit = 0.0,
                Level::Keep => {}
            }
            true
        } else if let Some(ord) = g.pending.iter_mut().find(|o| o.ticket == ticket) {
            price_open = ord.price_open;
            match stop_loss {
                Level::Set(v) => ord.stop_loss = v,
                Level::Clear => ord.stop_loss = 0.0,
                Level::Keep => {}
            }
            match take_profit {
                Level::Set(v) => ord.take_profit = v,
                Level::Clear => ord.take_profit = 0.0,
                Level::Keep => {}
            }
            true
        } else {
            false
        };
        if !modified {
            return Err(Mt5Error::Other(format!("ticket {ticket} not found")));
        }
        Ok(TradeResult {
            retcode: 10009,
            deal: 0,
            order: ticket,
            position: ticket,
            volume: 0.0,
            price: price_open,
        })
    }

    fn positions_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        Ok(self.positions())
    }

    fn pending_orders_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        let g = self.state.lock().unwrap();
        Ok(g.pending
            .iter()
            .map(|o| WorkingOrder {
                ticket: o.ticket,
                time_setup: Utc::now().timestamp(),
                order_type: o.order_type,
                magic: o.magic,
                volume_initial: o.volume_initial,
                volume_current: o.volume_current,
                price_open: o.price_open,
                stop_loss: o.stop_loss,
                take_profit: o.take_profit,
                price_current: o.price_current,
                symbol: o.symbol.clone(),
                comment: String::new(),
            })
            .collect())
    }

    fn deals_filtered(
        &self,
        _from: i64,
        _to: i64,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        let g = self.state.lock().unwrap();
        Ok(g.deals
            .iter()
            .map(|d| Deal {
                ticket: d.ticket,
                order: d.order,
                position_id: d.position_id,
                time: d.time,
                direction: d.direction,
                entry: d.entry,
                magic: d.magic,
                volume: d.volume,
                price: d.price,
                commission: 0.0,
                swap: 0.0,
                profit: 0.0,
                symbol: d.symbol.clone(),
                comment: String::new(),
            })
            .collect())
    }

    fn account_info(&self) -> Result<AccountInfo> {
        Ok(self.account())
    }
}

// =============================================================================
// Replay backend — reconstructs state from a capture or WAL and exposes the same
// [`TradingBackend`] trait, so incidents can be reproduced offline.
// =============================================================================

/// A replayable record of an executed order, produced by capturing a live session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayOrder {
    pub client_order_id: String,
    pub symbol: String,
    pub order_type: OrderType,
    pub volume: f64,
    pub price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub filled: bool,
    pub fill_price: f64,
    pub fill_time: i64,
}

/// Replay input: a sequence of recorded orders to replay against the backend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplayInput {
    pub orders: Vec<ReplayOrder>,
}

/// Replay backend: replays a recorded session through [`TradingBackend`].
///
/// Orders are replayed in sequence; each submit is answered from the recorded fill. No real
/// orders are placed.
#[derive(Debug, Default)]
pub struct ReplayBackend {
    input: Mutex<ReplayInput>,
    state: Mutex<ReplayState>,
}

#[derive(Debug, Default)]
struct ReplayState {
    next_ticket: u64,
    positions: Vec<DryPosition>,
    pending: Vec<DryWorkingOrder>,
    deals: Vec<DryDeal>,
    balance: f64,
    equity: f64,
    index: usize,
}

impl ReplayBackend {
    /// Create a replay backend from a [`ReplayInput`].
    pub fn new(input: ReplayInput) -> Self {
        let balance = input.orders.iter().fold(0.0, |acc, o| {
            acc + if o.filled {
                o.volume * o.fill_price
            } else {
                0.0
            }
        });
        Self {
            input: Mutex::new(input),
            state: Mutex::new(ReplayState {
                balance,
                ..Default::default()
            }),
        }
    }

    /// Advance one recorded order: return the order and whether it was filled.
    pub fn next(&self) -> Option<ReplayOrder> {
        let mut g = self.state.lock().unwrap();
        let input = self.input.lock().unwrap();
        if g.index >= input.orders.len() {
            return None;
        }
        let order = input.orders[g.index].clone();
        g.index += 1;
        drop(input);
        Some(order)
    }

    /// Number of orders remaining to replay.
    pub fn remaining(&self) -> usize {
        let input = self.input.lock().unwrap();
        let g = self.state.lock().unwrap();
        input.orders.len().saturating_sub(g.index)
    }

    /// Whether all recorded orders have been replayed.
    pub fn is_finished(&self) -> bool {
        self.remaining() == 0
    }
}

impl TradingBackend for ReplayBackend {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        let input = self.input.lock().unwrap();
        if g.index >= input.orders.len() {
            return Err(Mt5Error::Other("no more orders to replay".to_string()));
        }
        let recorded = input.orders[g.index].clone();
        drop(input);
        g.index += 1;

        let ticket = g.next_ticket;
        g.next_ticket += 1;

        if recorded.filled {
            let position_ticket = g.next_ticket;
            g.next_ticket += 1;
            let deal_ticket = g.next_ticket;
            g.next_ticket += 1;
            g.positions.push(DryPosition {
                ticket: position_ticket,
                symbol: recorded.symbol.clone(),
                position_type: recorded.order_type,
                volume: recorded.volume,
                price_open: recorded.fill_price,
                stop_loss: recorded.stop_loss,
                take_profit: recorded.take_profit,
                price_current: recorded.fill_price,
                magic: req.magic.unwrap_or(0),
            });
            g.deals.push(DryDeal {
                ticket: deal_ticket,
                order: ticket,
                position_id: position_ticket,
                time: recorded.fill_time,
                direction: recorded.order_type,
                entry: DealEntry::In,
                magic: req.magic.unwrap_or(0),
                volume: recorded.volume,
                price: recorded.fill_price,
                symbol: recorded.symbol.clone(),
            });
            g.balance -= recorded.volume * recorded.fill_price;
            g.equity = g.balance;
            Ok(TradeResult {
                retcode: 10009,
                deal: deal_ticket,
                order: ticket,
                position: position_ticket,
                volume: recorded.volume,
                price: recorded.fill_price,
            })
        } else {
            let working_ticket = g.next_ticket;
            g.next_ticket += 1;
            g.pending.push(DryWorkingOrder {
                ticket: working_ticket,
                symbol: recorded.symbol.clone(),
                order_type: recorded.order_type,
                volume_initial: recorded.volume,
                volume_current: recorded.volume,
                price_open: recorded.price,
                stop_loss: recorded.stop_loss,
                take_profit: recorded.take_profit,
                price_current: recorded.price,
                magic: req.magic.unwrap_or(0),
            });
            Ok(TradeResult {
                retcode: 10008,
                deal: 0,
                order: working_ticket,
                position: 0,
                volume: 0.0,
                price: recorded.price,
            })
        }
    }

    fn order_close_with_magic(&self, ticket: u64, _magic: u64) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        let Some(i) = g.positions.iter().position(|p| p.ticket == ticket) else {
            return Err(Mt5Error::Other(format!("position {ticket} not found")));
        };
        let p = g.positions.remove(i);
        let deal_ticket = g.next_ticket;
        g.next_ticket += 1;
        g.deals.push(DryDeal {
            ticket: deal_ticket,
            order: 0,
            position_id: ticket,
            time: Utc::now().timestamp(),
            direction: if p.position_type.is_buy() {
                OrderType::Sell
            } else {
                OrderType::Buy
            },
            entry: DealEntry::Out,
            magic: p.magic,
            volume: p.volume,
            price: p.price_current,
            symbol: p.symbol.clone(),
        });
        g.balance += p.volume * p.price_current;
        g.equity = g.balance;
        Ok(TradeResult {
            retcode: 10009,
            deal: deal_ticket,
            order: 0,
            position: ticket,
            volume: p.volume,
            price: p.price_current,
        })
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        _magic: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        let mut g = self.state.lock().unwrap();
        let mut price_open = 0.0f64;
        let modified = if let Some(pos) = g.positions.iter_mut().find(|p| p.ticket == ticket) {
            price_open = pos.price_open;
            match stop_loss {
                Level::Set(v) => pos.stop_loss = v,
                Level::Clear => pos.stop_loss = 0.0,
                Level::Keep => {}
            }
            match take_profit {
                Level::Set(v) => pos.take_profit = v,
                Level::Clear => pos.take_profit = 0.0,
                Level::Keep => {}
            }
            true
        } else if let Some(ord) = g.pending.iter_mut().find(|o| o.ticket == ticket) {
            price_open = ord.price_open;
            match stop_loss {
                Level::Set(v) => ord.stop_loss = v,
                Level::Clear => ord.stop_loss = 0.0,
                Level::Keep => {}
            }
            match take_profit {
                Level::Set(v) => ord.take_profit = v,
                Level::Clear => ord.take_profit = 0.0,
                Level::Keep => {}
            }
            true
        } else {
            false
        };
        if !modified {
            return Err(Mt5Error::Other(format!("ticket {ticket} not found")));
        }
        Ok(TradeResult {
            retcode: 10009,
            deal: 0,
            order: ticket,
            position: ticket,
            volume: 0.0,
            price: price_open,
        })
    }

    fn positions_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        let g = self.state.lock().unwrap();
        Ok(g.positions
            .iter()
            .map(|p| Position {
                ticket: p.ticket,
                time: Utc::now().timestamp(),
                position_type: p.position_type,
                magic: p.magic,
                volume: p.volume,
                price_open: p.price_open,
                stop_loss: p.stop_loss,
                take_profit: p.take_profit,
                price_current: p.price_current,
                profit: 0.0,
                swap: 0.0,
                symbol: p.symbol.clone(),
                comment: String::new(),
            })
            .collect())
    }

    fn pending_orders_filtered(
        &self,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        let g = self.state.lock().unwrap();
        Ok(g.pending
            .iter()
            .map(|o| WorkingOrder {
                ticket: o.ticket,
                time_setup: Utc::now().timestamp(),
                order_type: o.order_type,
                magic: o.magic,
                volume_initial: o.volume_initial,
                volume_current: o.volume_current,
                price_open: o.price_open,
                stop_loss: o.stop_loss,
                take_profit: o.take_profit,
                price_current: o.price_current,
                symbol: o.symbol.clone(),
                comment: String::new(),
            })
            .collect())
    }

    fn deals_filtered(
        &self,
        _from: i64,
        _to: i64,
        _magic: Option<u64>,
        _symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        let g = self.state.lock().unwrap();
        Ok(g.deals
            .iter()
            .map(|d| Deal {
                ticket: d.ticket,
                order: d.order,
                position_id: d.position_id,
                time: d.time,
                direction: d.direction,
                entry: d.entry,
                magic: d.magic,
                volume: d.volume,
                price: d.price,
                commission: 0.0,
                swap: 0.0,
                profit: 0.0,
                symbol: d.symbol.clone(),
                comment: String::new(),
            })
            .collect())
    }

    fn account_info(&self) -> Result<AccountInfo> {
        let g = self.state.lock().unwrap();
        Ok(AccountInfo {
            balance: g.balance,
            equity: g.equity,
            margin: 0.0,
            free_margin: g.balance,
            leverage: 0.0,
            currency: String::new(),
        })
    }
}

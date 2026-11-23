//! Python bindings for mt5-bridge via PyO3.
//!
//! Exposes the MetaTrader 5 bridge client (sync) to Python through PyO3.
//! Requires `mt5_bridge.dll` and the `mt5_bridge.mq5` Expert Advisor running
//! inside MetaTrader 5.
//!
//! ```python
//! import mt5_bridge_py
//! client = mt5_bridge_py.Client.connect_with_secret("pipe-secret")
//! account = client.account_info()
//! print(account.balance, account.equity)
//! ```

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PyType};
use pyo3::Py;

use mt5_bridge::client::Mt5Client;
use mt5_bridge::error::Mt5Error;
use mt5_bridge::types::{
    AccountInfo, Bar, BridgeStats, DealEntry, Level, OrderRequest, OrderType, PingResult,
    SymbolInfo, Tick, Timeframe, TradeResult,
};
use mt5_bridge::{CalendarEvent, Deal, Position, WorkingOrder};
use std::os::raw::c_int;
use std::str::FromStr;
use std::sync::OnceLock;

#[cfg(feature = "async")]
use std::sync::Arc;
#[cfg(feature = "async")]
use tokio::runtime::Runtime;
#[cfg(feature = "async")]
use tokio::sync::broadcast;

/// Static storage for the registered Python event callback.
/// `Py<PyAny>` is `Send` (it's just a raw pointer), so this is safe to access
/// from the DLL's callback thread as long as the GIL is held when dereferencing.
static EVENT_CALLBACK: OnceLock<std::sync::Mutex<Option<Py<PyAny>>>> = OnceLock::new();

#[pyclass]
#[derive(Clone)]
pub struct PyAccountInfo {
    #[pyo3(get, set)]
    pub balance: f64,
    #[pyo3(get, set)]
    pub equity: f64,
    #[pyo3(get, set)]
    pub margin: f64,
    #[pyo3(get, set)]
    pub free_margin: f64,
    #[pyo3(get, set)]
    pub leverage: f64,
    #[pyo3(get, set)]
    pub currency: String,
}

#[pymethods]
impl PyAccountInfo {
    #[new]
    fn new(
        balance: f64,
        equity: f64,
        margin: f64,
        free_margin: f64,
        leverage: f64,
        currency: &str,
    ) -> Self {
        Self {
            balance,
            equity,
            margin,
            free_margin,
            leverage,
            currency: currency.to_string(),
        }
    }

    fn profit(&self) -> f64 {
        self.equity - self.balance
    }

    fn margin_level(&self) -> Option<f64> {
        if self.margin > 0.0 {
            Some((self.equity / self.margin) * 100.0)
        } else {
            None
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "AccountInfo(balance={}, equity={}, margin={}, free_margin={}, leverage={}, currency={})",
            self.balance, self.equity, self.margin, self.free_margin, self.leverage, self.currency
        )
    }
}

impl From<AccountInfo> for PyAccountInfo {
    fn from(a: AccountInfo) -> Self {
        Self {
            balance: a.balance,
            equity: a.equity,
            margin: a.margin,
            free_margin: a.free_margin,
            leverage: a.leverage,
            currency: a.currency,
        }
    }
}

#[pyclass]
#[derive(Clone, Default)]
pub struct PySymbolInfo {
    #[pyo3(get, set)]
    pub symbol: String,
    #[pyo3(get, set)]
    pub point: f64,
    #[pyo3(get, set)]
    pub tick_value: f64,
    #[pyo3(get, set)]
    pub tick_size: f64,
    #[pyo3(get, set)]
    pub lot_step: f64,
    #[pyo3(get, set)]
    pub min_lot: f64,
    #[pyo3(get, set)]
    pub max_lot: f64,
    #[pyo3(get, set)]
    pub spread: f64,
    #[pyo3(get, set)]
    pub digits: u32,
    #[pyo3(get, set)]
    pub bid: f64,
    #[pyo3(get, set)]
    pub ask: f64,
}

#[pymethods]
impl PySymbolInfo {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "SymbolInfo(symbol={}, bid={}, ask={}, digits={})",
            self.symbol, self.bid, self.ask, self.digits
        )
    }
}

impl From<SymbolInfo> for PySymbolInfo {
    fn from(s: SymbolInfo) -> Self {
        Self {
            symbol: s.symbol,
            point: s.point,
            tick_value: s.tick_value,
            tick_size: s.tick_size,
            lot_step: s.lot_step,
            min_lot: s.min_lot,
            max_lot: s.max_lot,
            spread: s.spread,
            digits: s.digits,
            bid: s.bid,
            ask: s.ask,
        }
    }
}

#[pyclass]
#[derive(Clone, Default)]
pub struct PyTick {
    #[pyo3(get, set)]
    pub symbol: String,
    #[pyo3(get, set)]
    pub time_msc: i64,
    #[pyo3(get, set)]
    pub time: i64,
    #[pyo3(get, set)]
    pub bid: f64,
    #[pyo3(get, set)]
    pub ask: f64,
    #[pyo3(get, set)]
    pub last: f64,
    #[pyo3(get, set)]
    pub volume: u64,
    #[pyo3(get, set)]
    pub flags: u32,
    #[pyo3(get, set)]
    pub event_seq: u64,
}

#[pymethods]
impl PyTick {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn spread(&self) -> f64 {
        self.ask - self.bid
    }

    fn mid(&self) -> f64 {
        (self.ask + self.bid) / 2.0
    }

    fn __repr__(&self) -> String {
        format!(
            "Tick(symbol={}, bid={}, ask={}, time_msc={})",
            self.symbol, self.bid, self.ask, self.time_msc
        )
    }
}

impl From<Tick> for PyTick {
    fn from(t: Tick) -> Self {
        Self {
            symbol: t.symbol,
            time_msc: t.time_msc,
            time: t.time,
            bid: t.bid,
            ask: t.ask,
            last: t.last,
            volume: t.volume,
            flags: t.flags,
            event_seq: t.event_seq,
        }
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyBar {
    #[pyo3(get, set)]
    pub time: i64,
    #[pyo3(get, set)]
    pub open: f64,
    #[pyo3(get, set)]
    pub high: f64,
    #[pyo3(get, set)]
    pub low: f64,
    #[pyo3(get, set)]
    pub close: f64,
    #[pyo3(get, set)]
    pub volume: f64,
}

#[pymethods]
impl PyBar {
    #[new]
    fn new(time: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Self {
            time,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn mid(&self) -> f64 {
        (self.high + self.low) / 2.0
    }

    fn typical_price(&self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }

    fn range(&self) -> f64 {
        self.high - self.low
    }

    fn is_bullish(&self) -> bool {
        self.close > self.open
    }

    fn __repr__(&self) -> String {
        format!(
            "Bar(time={}, open={}, high={}, low={}, close={}, volume={})",
            self.time, self.open, self.high, self.low, self.close, self.volume
        )
    }
}

impl From<Bar> for PyBar {
    fn from(b: Bar) -> Self {
        Self {
            time: b.time,
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
        }
    }
}

#[pyclass]
#[derive(Clone, Copy)]
pub struct PyPingResult {
    #[pyo3(get, set)]
    pub server_time_secs: i64,
    #[pyo3(get, set)]
    pub protocol: u32,
    #[pyo3(get, set)]
    pub monotonic_microseconds: u64,
    #[pyo3(get, set)]
    pub rtt_us: u64,
}

#[pymethods]
impl PyPingResult {
    #[new]
    fn new(server_time_secs: i64, protocol: u32, monotonic_microseconds: u64, rtt_us: u64) -> Self {
        Self {
            server_time_secs,
            protocol,
            monotonic_microseconds,
            rtt_us,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "PingResult(server_time_secs={}, protocol={}, rtt_us={})",
            self.server_time_secs, self.protocol, self.rtt_us
        )
    }
}

impl From<PingResult> for PyPingResult {
    fn from(p: PingResult) -> Self {
        Self {
            server_time_secs: p.server_time_secs,
            protocol: p.protocol,
            monotonic_microseconds: p.monotonic_microseconds,
            rtt_us: p.rtt_us,
        }
    }
}

#[pyclass]
#[derive(Clone, Default)]
pub struct PyBridgeStats {
    #[pyo3(get, set)]
    pub uptime_secs: u64,
    #[pyo3(get, set)]
    pub requests_total: u64,
    #[pyo3(get, set)]
    pub tick_drops: u64,
    #[pyo3(get, set)]
    pub trade_drops: u64,
    #[pyo3(get, set)]
    pub book_drops: u64,
    #[pyo3(get, set)]
    pub bar_drops: u64,
    #[pyo3(get, set)]
    pub tick_queue_depth: u32,
    #[pyo3(get, set)]
    pub trade_queue_depth: u32,
    #[pyo3(get, set)]
    pub last_error: i32,
    #[pyo3(get, set)]
    pub protocol: u32,
    #[pyo3(get, set)]
    pub latency_p50_us: u64,
    #[pyo3(get, set)]
    pub latency_p95_us: u64,
    #[pyo3(get, set)]
    pub latency_p99_us: u64,
}

#[pymethods]
impl PyBridgeStats {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "BridgeStats(uptime_secs={}, requests_total={}, protocol={})",
            self.uptime_secs, self.requests_total, self.protocol
        )
    }
}

impl From<BridgeStats> for PyBridgeStats {
    fn from(s: BridgeStats) -> Self {
        Self {
            uptime_secs: s.uptime_secs,
            requests_total: s.requests_total,
            tick_drops: s.tick_drops,
            trade_drops: s.trade_drops,
            book_drops: s.book_drops,
            bar_drops: s.bar_drops,
            tick_queue_depth: s.tick_queue_depth,
            trade_queue_depth: s.trade_queue_depth,
            last_error: s.last_error,
            protocol: s.protocol,
            latency_p50_us: s.latency_p50_us,
            latency_p95_us: s.latency_p95_us,
            latency_p99_us: s.latency_p99_us,
        }
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyTradeResult {
    #[pyo3(get, set)]
    pub retcode: u32,
    #[pyo3(get, set)]
    pub deal: u64,
    #[pyo3(get, set)]
    pub order: u64,
    #[pyo3(get, set)]
    pub position: u64,
    #[pyo3(get, set)]
    pub volume: f64,
    #[pyo3(get, set)]
    pub price: f64,
}

#[pymethods]
impl PyTradeResult {
    #[new]
    fn new(retcode: u32, deal: u64, order: u64, position: u64, volume: f64, price: f64) -> Self {
        Self {
            retcode,
            deal,
            order,
            position,
            volume,
            price,
        }
    }

    fn is_success(&self) -> bool {
        TradeResult {
            retcode: self.retcode,
            deal: self.deal,
            order: self.order,
            position: self.position,
            volume: self.volume,
            price: self.price,
        }
        .is_success()
    }

    fn is_filled(&self) -> bool {
        TradeResult {
            retcode: self.retcode,
            deal: self.deal,
            order: self.order,
            position: self.position,
            volume: self.volume,
            price: self.price,
        }
        .is_filled()
    }

    fn is_placed(&self) -> bool {
        TradeResult {
            retcode: self.retcode,
            deal: self.deal,
            order: self.order,
            position: self.position,
            volume: self.volume,
            price: self.price,
        }
        .is_placed()
    }

    fn description(&self) -> String {
        TradeResult {
            retcode: self.retcode,
            deal: self.deal,
            order: self.order,
            position: self.position,
            volume: self.volume,
            price: self.price,
        }
        .description()
        .to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "TradeResult(retcode={}, deal={}, order={}, position={}, volume={}, price={})",
            self.retcode, self.deal, self.order, self.position, self.volume, self.price
        )
    }
}

impl From<TradeResult> for PyTradeResult {
    fn from(t: TradeResult) -> Self {
        Self {
            retcode: t.retcode,
            deal: t.deal,
            order: t.order,
            position: t.position,
            volume: t.volume,
            price: t.price,
        }
    }
}

/// A streamed event from MT5: `(event_type, payload_bytes)`.
#[pyclass]
#[derive(Clone)]
pub struct PyEvent {
    #[pyo3(get, set)]
    pub event_type: u16,
    #[pyo3(get, set)]
    pub payload: Vec<u8>,
}

#[pymethods]
impl PyEvent {
    #[new]
    fn new(event_type: u16, payload: Vec<u8>) -> Self {
        Self {
            event_type,
            payload,
        }
    }

    fn __repr__(&self) -> String {
        format!("PyEvent(event_type={}, payload_len={})", self.event_type, self.payload.len())
    }
}

#[pyclass]
#[derive(Clone, Default)]
pub struct PyPosition {
    #[pyo3(get, set)]
    pub ticket: u64,
    #[pyo3(get, set)]
    pub time: i64,
    #[pyo3(get, set)]
    pub position_type: String,
    #[pyo3(get, set)]
    pub magic: u64,
    #[pyo3(get, set)]
    pub volume: f64,
    #[pyo3(get, set)]
    pub price_open: f64,
    #[pyo3(get, set)]
    pub stop_loss: f64,
    #[pyo3(get, set)]
    pub take_profit: f64,
    #[pyo3(get, set)]
    pub price_current: f64,
    #[pyo3(get, set)]
    pub profit: f64,
    #[pyo3(get, set)]
    pub swap: f64,
    #[pyo3(get, set)]
    pub symbol: String,
    #[pyo3(get, set)]
    pub comment: String,
}

#[pymethods]
impl PyPosition {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "Position(ticket={}, symbol={}, type={}, volume={}, profit={})",
            self.ticket, self.symbol, self.position_type, self.volume, self.profit
        )
    }
}

impl From<Position> for PyPosition {
    fn from(p: Position) -> Self {
        Self {
            ticket: p.ticket,
            time: p.time,
            position_type: if p.position_type.is_buy() {
                "buy".to_string()
            } else {
                "sell".to_string()
            },
            magic: p.magic,
            volume: p.volume,
            price_open: p.price_open,
            stop_loss: p.stop_loss,
            take_profit: p.take_profit,
            price_current: p.price_current,
            profit: p.profit,
            swap: p.swap,
            symbol: p.symbol,
            comment: p.comment,
        }
    }
}

#[pyclass]
#[derive(Clone, Default)]
pub struct PyWorkingOrder {
    #[pyo3(get, set)]
    pub ticket: u64,
    #[pyo3(get, set)]
    pub time_setup: i64,
    #[pyo3(get, set)]
    pub order_type: String,
    #[pyo3(get, set)]
    pub magic: u64,
    #[pyo3(get, set)]
    pub volume_initial: f64,
    #[pyo3(get, set)]
    pub volume_current: f64,
    #[pyo3(get, set)]
    pub price_open: f64,
    #[pyo3(get, set)]
    pub stop_loss: f64,
    #[pyo3(get, set)]
    pub take_profit: f64,
    #[pyo3(get, set)]
    pub price_current: f64,
    #[pyo3(get, set)]
    pub symbol: String,
    #[pyo3(get, set)]
    pub comment: String,
}

#[pymethods]
impl PyWorkingOrder {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "WorkingOrder(ticket={}, symbol={}, type={}, price_open={})",
            self.ticket, self.symbol, self.order_type, self.price_open
        )
    }
}

impl From<WorkingOrder> for PyWorkingOrder {
    fn from(w: WorkingOrder) -> Self {
        Self {
            ticket: w.ticket,
            time_setup: w.time_setup,
            order_type: if w.order_type.is_buy() {
                "buy".to_string()
            } else {
                "sell".to_string()
            },
            magic: w.magic,
            volume_initial: w.volume_initial,
            volume_current: w.volume_current,
            price_open: w.price_open,
            stop_loss: w.stop_loss,
            take_profit: w.take_profit,
            price_current: w.price_current,
            symbol: w.symbol,
            comment: w.comment,
        }
    }
}

#[pyclass]
#[derive(Clone, Default)]
pub struct PyDeal {
    #[pyo3(get, set)]
    pub ticket: u64,
    #[pyo3(get, set)]
    pub order: u64,
    #[pyo3(get, set)]
    pub position_id: u64,
    #[pyo3(get, set)]
    pub time: i64,
    #[pyo3(get, set)]
    pub direction: String,
    #[pyo3(get, set)]
    pub entry: String,
    #[pyo3(get, set)]
    pub magic: u64,
    #[pyo3(get, set)]
    pub volume: f64,
    #[pyo3(get, set)]
    pub price: f64,
    #[pyo3(get, set)]
    pub commission: f64,
    #[pyo3(get, set)]
    pub swap: f64,
    #[pyo3(get, set)]
    pub profit: f64,
    #[pyo3(get, set)]
    pub symbol: String,
    #[pyo3(get, set)]
    pub comment: String,
}

#[pymethods]
impl PyDeal {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "Deal(ticket={}, symbol={}, direction={}, volume={}, profit={})",
            self.ticket, self.symbol, self.direction, self.volume, self.profit
        )
    }
}

impl From<Deal> for PyDeal {
    fn from(d: Deal) -> Self {
        Self {
            ticket: d.ticket,
            order: d.order,
            position_id: d.position_id,
            time: d.time,
            direction: if d.direction.is_buy() {
                "buy".to_string()
            } else {
                "sell".to_string()
            },
            entry: match d.entry {
                DealEntry::In => "in".to_string(),
                DealEntry::Out => "out".to_string(),
                DealEntry::InOut => "in_out".to_string(),
                DealEntry::OutBy => "out_by".to_string(),
                DealEntry::Other => "other".to_string(),
            },
            magic: d.magic,
            volume: d.volume,
            price: d.price,
            commission: d.commission,
            swap: d.swap,
            profit: d.profit,
            symbol: d.symbol,
            comment: d.comment,
        }
    }
}

#[pyclass]
#[derive(Clone, Default)]
pub struct PyCalendarEvent {
    #[pyo3(get, set)]
    pub id: u64,
    #[pyo3(get, set)]
    pub time: i64,
    #[pyo3(get, set)]
    pub country: String,
    #[pyo3(get, set)]
    pub currency: String,
    #[pyo3(get, set)]
    pub importance: u8,
    #[pyo3(get, set)]
    pub title: String,
    #[pyo3(get, set)]
    pub actual: Option<f64>,
    #[pyo3(get, set)]
    pub forecast: Option<f64>,
    #[pyo3(get, set)]
    pub previous: Option<f64>,
}

#[pymethods]
impl PyCalendarEvent {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn is_released(&self) -> bool {
        self.actual.is_some()
    }

    fn __repr__(&self) -> String {
        format!(
            "CalendarEvent(id={}, time={}, title={}, importance={})",
            self.id, self.time, self.title, self.importance
        )
    }
}

impl From<CalendarEvent> for PyCalendarEvent {
    fn from(c: CalendarEvent) -> Self {
        Self {
            id: c.id,
            time: c.time,
            country: c.country,
            currency: c.currency,
            importance: c.importance,
            title: c.title,
            actual: c.actual,
            forecast: c.forecast,
            previous: c.previous,
        }
    }
}

#[pyclass]
struct Client {
    inner: Mt5Client,
}

#[pymethods]
impl Client {
    #[new]
    fn new() -> PyResult<Self> {
        let client = Mt5Client::connect_with_secret("").map_err(to_py_err)?;
        Ok(Client { inner: client })
    }

    #[classmethod]
    fn connect_with_secret(_cls: &Bound<'_, PyType>, secret: &str) -> PyResult<Self> {
        let client = Mt5Client::connect_with_secret(secret).map_err(to_py_err)?;
        Ok(Client { inner: client })
    }

    #[classmethod]
    fn connect(
        _cls: &Bound<'_, PyType>,
        login: i64,
        password: &str,
        server: &str,
    ) -> PyResult<Self> {
        let client = Mt5Client::connect(login, password, server).map_err(to_py_err)?;
        Ok(Client { inner: client })
    }

    fn shutdown(&mut self) -> PyResult<()> {
        self.inner.shutdown().map_err(to_py_err)
    }

    fn account_info<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAccountInfo>> {
        let info = self.inner.account_info().map_err(to_py_err)?;
        Ok(PyAccountInfo::from(info).into_pyobject(py)?.unbind())
    }

    fn symbol_info<'py>(&self, py: Python<'py>, symbol: &str) -> PyResult<Py<PySymbolInfo>> {
        let info = self.inner.symbol_info(symbol).map_err(to_py_err)?;
        Ok(PySymbolInfo::from(info).into_pyobject(py)?.unbind())
    }

    fn symbol_tick<'py>(&self, py: Python<'py>, symbol: &str) -> PyResult<Py<PyTick>> {
        let tick = self.inner.symbol_tick(symbol).map_err(to_py_err)?;
        Ok(PyTick::from(tick).into_pyobject(py)?.unbind())
    }

    fn copy_bars<'py>(
        &self,
        py: Python<'py>,
        symbol: &str,
        timeframe: &str,
        from: i64,
        to: i64,
    ) -> PyResult<Py<PyAny>> {
        let tf = Timeframe::from_str(timeframe).map_err(PyValueError::new_err)?;
        let bars = self
            .inner
            .copy_bars(symbol, tf, from, to)
            .map_err(to_py_err)?;
        let py_list = PyList::empty(py);
        for b in bars {
            py_list.append(PyBar::from(b))?;
        }
        Ok(py_list.into_any().unbind())
    }

    fn ping<'py>(&self, py: Python<'py>) -> PyResult<Py<PyPingResult>> {
        let result = self.inner.ping().map_err(to_py_err)?;
        Ok(PyPingResult::from(result).into_pyobject(py)?.unbind())
    }

    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Py<PyBridgeStats>> {
        let stats = self.inner.stats().map_err(to_py_err)?;
        Ok(PyBridgeStats::from(stats).into_pyobject(py)?.unbind())
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (symbol, order_type, volume, price, sl=0.0, tp=0.0, comment="", deviation=None, expiration=None, magic=None))]
    fn order_send<'py>(
        &self,
        py: Python<'py>,
        symbol: &str,
        order_type: &str,
        volume: f64,
        price: f64,
        sl: f64,
        tp: f64,
        comment: &str,
        deviation: Option<u32>,
        expiration: Option<i64>,
        magic: Option<u64>,
    ) -> PyResult<Py<PyTradeResult>> {
        let ot = match order_type {
            "buy" => OrderType::Buy,
            "sell" => OrderType::Sell,
            "buy_limit" => OrderType::BuyLimit,
            "sell_limit" => OrderType::SellLimit,
            "buy_stop" => OrderType::BuyStop,
            "sell_stop" => OrderType::SellStop,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unknown order type: {order_type}"
                )))
            }
        };
        let req = OrderRequest {
            symbol: symbol.to_string(),
            order_type: ot,
            volume,
            price,
            stop_loss: sl,
            take_profit: tp,
            comment: comment.to_string(),
            deviation,
            expiration,
            magic,
            client_order_id: None,
        };
        let result = self.inner.order_send(&req).map_err(to_py_err)?;
        Ok(PyTradeResult::from(result).into_pyobject(py)?.unbind())
    }

    fn order_close<'py>(&self, py: Python<'py>, ticket: u64) -> PyResult<Py<PyTradeResult>> {
        let result = self.inner.order_close(ticket).map_err(to_py_err)?;
        Ok(PyTradeResult::from(result).into_pyobject(py)?.unbind())
    }

    fn order_close_with_magic<'py>(
        &self,
        py: Python<'py>,
        ticket: u64,
        magic: u64,
    ) -> PyResult<Py<PyTradeResult>> {
        let result = self
            .inner
            .order_close_with_magic(ticket, magic)
            .map_err(to_py_err)?;
        Ok(PyTradeResult::from(result).into_pyobject(py)?.unbind())
    }

    fn order_modify<'py>(
        &self,
        py: Python<'py>,
        ticket: u64,
        sl_mode: &str,
        sl: f64,
        tp_mode: &str,
        tp: f64,
    ) -> PyResult<Py<PyTradeResult>> {
        let stop_loss = parse_level(sl_mode, sl)?;
        let take_profit = parse_level(tp_mode, tp)?;
        let result = self
            .inner
            .order_modify(ticket, stop_loss, take_profit)
            .map_err(to_py_err)?;
        Ok(PyTradeResult::from(result).into_pyobject(py)?.unbind())
    }

    fn order_modify_with_magic<'py>(
        &self,
        py: Python<'py>,
        ticket: u64,
        magic: u64,
        sl_level: &str,
        tp_level: &str,
    ) -> PyResult<Py<PyTradeResult>> {
        let stop_loss = parse_level_str(sl_level)?;
        let take_profit = parse_level_str(tp_level)?;
        let result = self
            .inner
            .order_modify_with_magic(ticket, magic, stop_loss, take_profit)
            .map_err(to_py_err)?;
        Ok(PyTradeResult::from(result).into_pyobject(py)?.unbind())
    }

    fn positions<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let list = self.inner.positions().map_err(to_py_err)?;
        let py_list = PyList::empty(py);
        for p in list {
            py_list.append(PyPosition::from(p))?;
        }
        Ok(py_list.into_any().unbind())
    }

    fn pending_orders<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let list = self.inner.pending_orders().map_err(to_py_err)?;
        let py_list = PyList::empty(py);
        for w in list {
            py_list.append(PyWorkingOrder::from(w))?;
        }
        Ok(py_list.into_any().unbind())
    }

    fn deals<'py>(&self, py: Python<'py>, from: i64, to: i64) -> PyResult<Py<PyAny>> {
        let list = self.inner.deals(from, to).map_err(to_py_err)?;
        let py_list = PyList::empty(py);
        for d in list {
            py_list.append(PyDeal::from(d))?;
        }
        Ok(py_list.into_any().unbind())
    }

    fn symbols_total(&self) -> PyResult<u32> {
        self.inner.symbols_total().map_err(to_py_err)
    }

    fn symbols_page<'py>(&self, py: Python<'py>, page: u32, page_size: u32) -> PyResult<Py<PyAny>> {
        let list = self
            .inner
            .symbols_page(page, page_size)
            .map_err(to_py_err)?;
        let py_list = PyList::empty(py);
        for s in list {
            py_list.append(PySymbolInfo::from(s))?;
        }
        Ok(py_list.into_any().unbind())
    }

    fn symbol_select(&self, symbol: &str, enable: bool) -> PyResult<()> {
        self.inner.symbol_select(symbol, enable).map_err(to_py_err)
    }

    fn calendar_events<'py>(&self, py: Python<'py>, from: u64, to: u64) -> PyResult<Py<PyAny>> {
        let list = self.inner.calendar_events(from, to).map_err(to_py_err)?;
        let py_list = PyList::empty(py);
        for c in list {
            py_list.append(PyCalendarEvent::from(c))?;
        }
        Ok(py_list.into_any().unbind())
    }

    fn calendar_countries<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let list = self.inner.calendar_countries().map_err(to_py_err)?;
        let py_list = PyList::empty(py);
        for c in list {
            py_list.append(c.as_str())?;
        }
        Ok(py_list.into_any().unbind())
    }

    #[cfg(feature = "async")]
    fn subscribe_ticks(&self, symbol: &str) -> PyResult<bool> {
        self.inner.subscribe_ticks(symbol).map_err(to_py_err)?;
        Ok(true)
    }

    #[cfg(feature = "async")]
    fn unsubscribe_ticks(&self, symbol: &str) -> PyResult<()> {
        self.inner.unsubscribe_ticks(symbol).map_err(to_py_err)
    }

    #[cfg(feature = "async")]
    fn subscribe_trade(&self) -> PyResult<bool> {
        self.inner.subscribe_trade().map_err(to_py_err)?;
        Ok(true)
    }

    #[cfg(feature = "async")]
    fn unsubscribe_trade(&self) -> PyResult<()> {
        self.inner.unsubscribe_trade().map_err(to_py_err)
    }

    #[cfg(feature = "async")]
    fn subscribe_book(&self, symbol: &str) -> PyResult<bool> {
        self.inner.subscribe_book(symbol).map_err(to_py_err)?;
        Ok(true)
    }

    #[cfg(feature = "async")]
    fn unsubscribe_book(&self, symbol: &str) -> PyResult<()> {
        self.inner.unsubscribe_book(symbol).map_err(to_py_err)
    }

    #[cfg(feature = "async")]
    fn subscribe_bars(&self, symbol: &str, timeframe: &str) -> PyResult<bool> {
        let tf = Timeframe::from_str(timeframe).map_err(PyValueError::new_err)?;
        self.inner.subscribe_bars(symbol, tf).map_err(to_py_err)?;
        Ok(true)
    }

    #[cfg(feature = "async")]
    fn unsubscribe_bars(&self, symbol: &str, timeframe: &str) -> PyResult<()> {
        let tf = Timeframe::from_str(timeframe).map_err(PyValueError::new_err)?;
        self.inner.unsubscribe_bars(symbol, tf).map_err(to_py_err)
    }
}

#[cfg(feature = "async")]
#[pyclass]
pub struct AsyncClient {
    inner: Arc<Mt5Client>,
    runtime: Runtime,
    events_tx: broadcast::Sender<PyEvent>,
}

#[cfg(feature = "async")]
#[pymethods]
impl AsyncClient {
    /// Connect to MetaTrader 5 using only a pipe authentication secret token,
    /// running the blocking connect on a Tokio worker thread.
    #[new]
    fn new() -> PyResult<Self> {
        let runtime = Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("failed to create Tokio runtime: {e}")))?;
        let client = runtime
            .block_on(async {
                tokio::task::spawn_blocking(move || {
                    Mt5Client::connect_with_secret("").map_err(to_py_err)
                })
                .await
            })
            .map_err(|e| PyRuntimeError::new_err(format!("async task join error: {e}")))??;
        let (events_tx, _) = broadcast::channel(1024);
        let client_arc = Arc::new(client);
        let client_arc_struct = Arc::clone(&client_arc);
        let tx = events_tx.clone();
        runtime.spawn(async move {
            loop {
                let inner = Arc::clone(&client_arc);
                let tx = tx.clone();
                match tokio::task::spawn_blocking(move || inner.poll_event(100)).await {
                    Ok(Ok(Some((event_type, payload)))) => {
                        let _ = tx.send(PyEvent::new(event_type, payload));
                    }
                    _ => {}
                }
            }
        });
        Ok(AsyncClient {
            inner: client_arc_struct,
            runtime,
            events_tx,
        })
    }

    /// Connect to MetaTrader 5 using a pipe authentication secret token.
    #[classmethod]
    fn connect_with_secret(_cls: &Bound<'_, PyType>, secret: &str) -> PyResult<Self> {
        let runtime = Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("failed to create Tokio runtime: {e}")))?;
        let client = runtime
            .block_on(async {
                let secret = secret.to_string();
                tokio::task::spawn_blocking(move || {
                    Mt5Client::connect_with_secret(&secret).map_err(to_py_err)
                })
                .await
            })
            .map_err(|e| PyRuntimeError::new_err(format!("async task join error: {e}")))??;
        let (events_tx, _) = broadcast::channel(1024);
        let client_arc = Arc::new(client);
        let client_arc_struct = Arc::clone(&client_arc);
        let tx = events_tx.clone();
        runtime.spawn(async move {
            loop {
                let inner = Arc::clone(&client_arc);
                let tx = tx.clone();
                match tokio::task::spawn_blocking(move || inner.poll_event(100)).await {
                    Ok(Ok(Some((event_type, payload)))) => {
                        let _ = tx.send(PyEvent::new(event_type, payload));
                    }
                    _ => {}
                }
            }
        });
        Ok(AsyncClient {
            inner: client_arc_struct,
            runtime,
            events_tx,
        })
    }

    /// Query current account balance, equity, and margin information.
    async fn account_info(&self) -> PyResult<PyAccountInfo> {
        let inner = Arc::clone(&self.inner);
        let info = self.run_blocking(move || inner.account_info())?;
        Ok(PyAccountInfo::from(info))
    }

    /// Symbol properties (point size, tick value, tick size, lot limits, digits).
    async fn symbol_info(&self, symbol: &str) -> PyResult<PySymbolInfo> {
        let inner = Arc::clone(&self.inner);
        let symbol = symbol.to_string();
        let info = self.run_blocking(move || inner.symbol_info(&symbol))?;
        Ok(PySymbolInfo::from(info))
    }

    /// Latest tick for a symbol.
    async fn symbol_tick(&self, symbol: &str) -> PyResult<PyTick> {
        let inner = Arc::clone(&self.inner);
        let symbol = symbol.to_string();
        let tick = self.run_blocking(move || inner.symbol_tick(&symbol))?;
        Ok(PyTick::from(tick))
    }

    /// Fetch historical OHLCV bars for `symbol` on `timeframe` between `from` and `to` (UTC seconds).
    async fn copy_bars(
        &self,
        symbol: &str,
        timeframe: &str,
        from: i64,
        to: i64,
    ) -> PyResult<Vec<PyBar>> {
        let inner = Arc::clone(&self.inner);
        let symbol = symbol.to_string();
        let tf = Timeframe::from_str(timeframe).map_err(PyValueError::new_err)?;
        let bars = self.run_blocking(move || inner.copy_bars(&symbol, tf, from, to))?;
        Ok(bars.into_iter().map(PyBar::from).collect())
    }

    /// Heartbeat ping returning MT5 server time, monotonic microsecond counter, and protocol version.
    async fn ping(&self) -> PyResult<PyPingResult> {
        let inner = Arc::clone(&self.inner);
        let result = self.run_blocking(move || inner.ping())?;
        Ok(PyPingResult::from(result))
    }

    /// Bridge statistics snapshot.
    async fn stats(&self) -> PyResult<PyBridgeStats> {
        let inner = Arc::clone(&self.inner);
        let stats = self.run_blocking(move || inner.stats())?;
        Ok(PyBridgeStats::from(stats))
    }

    /// Open or place an order.
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (symbol, order_type, volume, price, sl=0.0, tp=0.0, comment="", deviation=None, expiration=None, magic=None))]
    async fn order_send(
        &self,
        symbol: &str,
        order_type: &str,
        volume: f64,
        price: f64,
        sl: f64,
        tp: f64,
        comment: &str,
        deviation: Option<u32>,
        expiration: Option<i64>,
        magic: Option<u64>,
    ) -> PyResult<PyTradeResult> {
        let inner = Arc::clone(&self.inner);
        let ot = match order_type {
            "buy" => OrderType::Buy,
            "sell" => OrderType::Sell,
            "buy_limit" => OrderType::BuyLimit,
            "sell_limit" => OrderType::SellLimit,
            "buy_stop" => OrderType::BuyStop,
            "sell_stop" => OrderType::SellStop,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unknown order type: {order_type}"
                )))
            }
        };
        let symbol = symbol.to_string();
        let comment = comment.to_string();
        let result = self.run_blocking(move || {
            let req = OrderRequest {
                symbol,
                order_type: ot,
                volume,
                price,
                stop_loss: sl,
                take_profit: tp,
                comment,
                deviation,
                expiration,
                magic,
                client_order_id: None,
            };
            inner.order_send(&req)
        })?;
        Ok(PyTradeResult::from(result))
    }

    /// Close position by ticket.
    async fn order_close(&self, ticket: u64) -> PyResult<PyTradeResult> {
        let inner = Arc::clone(&self.inner);
        let result = self.run_blocking(move || inner.order_close(ticket))?;
        Ok(PyTradeResult::from(result))
    }

    /// Disconnect and clean up.
    async fn shutdown(&mut self) -> PyResult<()> {
        let inner = Arc::clone(&self.inner);
        self.run_blocking(move || inner.shutdown())
    }

    /// Return an async event stream: `async for event in client.events(): ...`.
    /// Yields `PyEvent(event_type, payload)` for each streamed tick/bar/trade
    /// event delivered by the DLL.
    fn events(&self) -> PyResult<PyEventStream> {
        let rx = self.events_tx.subscribe();
        Ok(PyEventStream { rx })
    }

    /// Register a Python callable `(event_type: int, payload: bytes) -> None`
    /// invoked by the DLL whenever a streamed event arrives.
    #[cfg(feature = "async")]
    fn register_event_callback(&self, callback: Py<PyAny>) -> PyResult<c_int> {
        let slot = EVENT_CALLBACK.get_or_init(|| std::sync::Mutex::new(None));
        let Ok(mut guard) = slot.lock() else {
            return Err(PyRuntimeError::new_err("event callback lock poisoned"));
        };
        *guard = Some(callback);

        extern "C" fn trampoline(event_type: u16, payload: *const u8, payload_len: u32) {
            let slot = match EVENT_CALLBACK.get() {
                Some(s) => s,
                None => return,
            };
            let Ok(guard) = slot.lock() else {
                return;
            };
            let Some(cb) = guard.as_ref() else {
                return;
            };
            let payload_slice = unsafe { std::slice::from_raw_parts(payload, payload_len as usize) };
            let _ = pyo3::Python::try_attach(|py| {
                let args = (event_type, PyBytes::new(py, payload_slice));
                let _ = cb.call1(py, args);
            });
        }

        self.inner
            .register_event_callback(trampoline)
            .map_err(to_py_err)
    }

    /// Poll the DLL for the next available event.
    ///
    /// Returns `(event_type, payload)` or `None` on timeout.
    #[cfg(feature = "async")]
    fn poll_event(&self, py: Python<'_>, timeout_ms: u32) -> PyResult<Option<Py<PyEvent>>> {
        match self.inner.poll_event(timeout_ms).map_err(to_py_err)? {
            Some((event_type, payload)) => {
                Ok(Some(PyEvent::new(event_type, payload).into_pyobject(py)?.unbind()))
            }
            None => Ok(None),
        }
    }

    /// Total events dropped by the DLL buffer queue due to overflow.
    #[cfg(feature = "async")]
    fn events_dropped_total(&self) -> PyResult<u64> {
        Ok(self.inner.events_dropped_total())
    }

    /// Total orphaned events currently held by the DLL event queue.
    #[cfg(feature = "async")]
    fn get_orphan_count(&self) -> PyResult<u64> {
        Ok(self.inner.get_orphan_count())
    }
}

#[cfg(feature = "async")]
impl AsyncClient {
    /// Run a blocking `Mt5Client` call off the Python thread on Tokio's
    /// `spawn_blocking` pool, awaiting the result without blocking the caller's
    /// async context beyond the `block_on` wait on this thread.
    fn run_blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce() -> Result<T, Mt5Error> + Send + 'static,
    ) -> Result<T, PyErr> {
        let res = self
            .runtime
            .block_on(async { tokio::task::spawn_blocking(f).await })
            .map_err(|e| PyRuntimeError::new_err(format!("async task join error: {e}")))?;
        res.map_err(to_py_err)
    }
}

/// Async event stream returned by [`AsyncClient::events`]. Supports
/// `async for event in client.events(): ...`.
#[cfg(feature = "async")]
#[pyclass]
pub struct PyEventStream {
    rx: broadcast::Receiver<PyEvent>,
}

#[cfg(feature = "async")]
#[pymethods]
impl PyEventStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        Ok(slf)
    }

    fn __anext__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        loop {
            match self.rx.try_recv() {
                Ok(event) => {
                    let obj: Py<PyAny> = event.into_pyobject(py)?.unbind().into();
                    return Ok(Some(obj));
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    let obj: Py<PyAny> = PyEvent::new(0, vec![])
                        .into_pyobject(py)?
                        .unbind()
                        .into();
                    return Ok(Some(obj));
                }
                Err(broadcast::error::TryRecvError::Closed) => return Ok(None),
                Err(broadcast::error::TryRecvError::Empty) => {
                    if std::time::Instant::now() >= deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }
}

fn to_py_err(err: Mt5Error) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

fn parse_level(mode: &str, price: f64) -> PyResult<Level> {
    match mode {
        "keep" => Ok(Level::Keep),
        "clear" => Ok(Level::Clear),
        "set" => Ok(Level::Set(price)),
        _ => Err(PyValueError::new_err(format!(
            "unknown level mode: {mode} (expected 'keep', 'clear', or 'set')"
        ))),
    }
}

fn parse_level_str(s: &str) -> PyResult<Level> {
    match s {
        "keep" => Ok(Level::Keep),
        "clear" => Ok(Level::Clear),
        s if s.starts_with("set:") => {
            let v: f64 = s[4..]
                .parse()
                .map_err(|_| PyValueError::new_err(format!("invalid level value: {s}")))?;
            Ok(Level::Set(v))
        }
        _ => Err(PyValueError::new_err(format!(
            "invalid level: {s} (expected 'keep', 'clear', or 'set:<price>')"
        ))),
    }
}

#[pymodule]
fn mt5_bridge_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Client>()?;
    #[cfg(feature = "async")]
    m.add_class::<AsyncClient>()?;
    #[cfg(feature = "async")]
    m.add_class::<PyEventStream>()?;
    m.add_class::<PyAccountInfo>()?;
    m.add_class::<PySymbolInfo>()?;
    m.add_class::<PyTick>()?;
    m.add_class::<PyBar>()?;
    m.add_class::<PyEvent>()?;
    m.add_class::<PyPingResult>()?;
    m.add_class::<PyBridgeStats>()?;
    m.add_class::<PyTradeResult>()?;
    m.add_class::<PyPosition>()?;
    m.add_class::<PyWorkingOrder>()?;
    m.add_class::<PyDeal>()?;
    m.add_class::<PyCalendarEvent>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

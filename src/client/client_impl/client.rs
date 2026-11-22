use crate::error::{Mt5Error, Result};
use crate::ffi::*;
use crate::types::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Safe client interface to the MetaTrader 5 bridge DLL and EA server.
///
/// `Mt5Client` manages dynamic library loading, named pipe communication,
/// account querying, symbol specifications, historical data downloading,
/// and live trading.
///
/// Under the hood, all requests are serialized and sent over a Windows Named Pipe
/// (`\\.\pipe\mt5bridge` or the name configured in `MT5_PIPE_NAME`) to the
/// `mt5_bridge.mq5` Expert Advisor running inside MetaTrader 5.
///
/// # One client per process
///
/// Only one live `Mt5Client` may exist per process. The loaded DLL keeps the
/// pipe connection, subscriptions, and event state in process-global variables,
/// so a second [`Mt5Client::connect`] while a client is alive fails with
/// [`Mt5Error::AlreadyConnected`].
///
/// To talk to several MetaTrader terminals at the same time, run one process
/// per terminal and point each process at its own EA pipe through the
/// `MT5_PIPE_NAME` environment variable. The variable is read when the client
/// connects (and when the DLL initializes), not on every request; changing it
/// after a successful connect does not move the live connection.
///
/// ```text
/// MT5_PIPE_NAME=mt5bridge_account1 ./my_bot_account1
/// MT5_PIPE_NAME=mt5bridge_account2 ./my_bot_account2
/// ```
///
/// Dropping the client or calling [`Mt5Client::shutdown`] releases the slot, so
/// a later client can connect in the same process. Use
/// [`Mt5Client::process_connection_count`] to inspect the slot for diagnostics.
pub struct Mt5Client {
    pub(crate) _lib: libloading::Library,
    pub(crate) fn_init: FnInit,
    pub(crate) fn_shut: FnShut,
    pub(crate) fn_rates: FnRates,
    pub(crate) fn_acct: FnAcct,
    pub(crate) fn_send: FnSend,
    pub(crate) fn_close: FnClose,
    pub(crate) fn_close_magic: Option<FnCloseMagic>,
    pub(crate) fn_close_deviation: Option<FnCloseDeviation>,
    pub(crate) fn_modify: Option<FnModify>,
    pub(crate) fn_modify_magic: Option<FnModifyMagic>,
    pub(crate) fn_sym_tick: Option<FnSymTick>,
    pub(crate) fn_sym_info: Option<FnSymInfo>,
    pub(crate) fn_symbols_total: Option<FnSymbolsTotal>,
    pub(crate) fn_symbols_get: Option<FnSymbolsGet>,
    pub(crate) fn_symbol_select: Option<FnSymbolSelect>,
    pub(crate) fn_positions: Option<FnPositions>,
    pub(crate) fn_orders: Option<FnOrders>,
    pub(crate) fn_deals: Option<FnDeals>,
    pub(crate) fn_ping: Option<FnPing>,
    #[cfg(feature = "async")]
    pub(crate) fn_subscribe_ticks: Option<FnSubscribeTicks>,
    #[cfg(feature = "async")]
    pub(crate) fn_unsubscribe_ticks: Option<FnUnsubscribeTicks>,
    #[cfg(feature = "async")]
    pub(crate) fn_subscribe_trade: Option<FnSubscribeTrade>,
    #[cfg(feature = "async")]
    pub(crate) fn_unsubscribe_trade: Option<FnUnsubscribeTrade>,
    #[cfg(feature = "async")]
    pub(crate) fn_subscribe_book: Option<FnSubscribeBook>,
    #[cfg(feature = "async")]
    pub(crate) fn_unsubscribe_book: Option<FnUnsubscribeBook>,
    #[cfg(feature = "async")]
    pub(crate) fn_subscribe_bars: Option<FnSubscribeBars>,
    #[cfg(feature = "async")]
    pub(crate) fn_unsubscribe_bars: Option<FnUnsubscribeBars>,
    #[cfg(feature = "async")]
    pub(crate) fn_events_dropped_total: Option<FnEventsDroppedTotal>,
    pub(crate) symbol_cache: Arc<Mutex<HashMap<String, (SymbolInfo, Instant)>>>,
    pub(crate) symbol_cache_ttl: Duration,
    #[cfg(feature = "async")]
    pub(crate) event_bus: Arc<crate::stream::EventBus>,
    #[cfg(feature = "async")]
    pub(crate) event_loop_running: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "async")]
    pub(crate) event_pump_handle: Option<std::thread::JoinHandle<()>>,
    pub(crate) is_shutdown: std::sync::atomic::AtomicBool,
    pub(crate) session: Arc<Mutex<crate::session::Mt5Session>>,
    #[cfg(feature = "async")]
    pub(crate) disconnect_tx: tokio::sync::broadcast::Sender<()>,
}

// Safety: Mt5Client is safe to send and share across threads.
// All pipe I/O and state modifications are serialized via a Win32 CRITICAL_SECTION
// in the underlying C++ DLL. Note that because of this serialization, concurrent
// operations across threads (such as streaming ticks/bars and order execution) will
// serialize and contend on that single global mutex under the hood.
pub(crate) static CLIENT_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Clears `CLIENT_ACTIVE` if construction fails after the flag was claimed.
pub(crate) struct ActiveGuard {
    armed: bool,
}

impl ActiveGuard {
    pub(crate) fn arm() -> Self {
        Self { armed: true }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if self.armed {
            CLIENT_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

impl Mt5Client {
    /// Returns the number of live `Mt5Client` connections in this process.
    ///
    /// Only one client may be connected per process (see the [type-level
    /// documentation](Self)), so this returns either `0` or `1`. It is intended
    /// for diagnostics, health endpoints, and tests that need to confirm the
    /// process-wide connection slot is free.
    ///
    /// ```
    /// use mt5_bridge::Mt5Client;
    ///
    /// // Doctests run in a fresh process with no live client.
    /// assert_eq!(Mt5Client::process_connection_count(), 0);
    /// ```
    pub fn process_connection_count() -> usize {
        usize::from(CLIENT_ACTIVE.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// Session tracking connectivity, subscriptions, and reconciliation state.
    pub fn session(&self) -> Arc<Mutex<crate::session::Mt5Session>> {
        Arc::clone(&self.session)
    }

    /// Register a symbol tick subscription to be preserved across reconnects.
    pub fn register_tick_subscription(&self, symbol: &str) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .register_tick_subscription(symbol);
    }

    /// Unregister a symbol tick subscription.
    pub fn unregister_tick_subscription(&self, symbol: &str) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .unregister_tick_subscription(symbol);
    }

    /// Register trade event subscription to be preserved across reconnects.
    pub fn register_trade_subscription(&self) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .register_trade_subscription();
    }

    /// Unregister trade event subscription.
    pub fn unregister_trade_subscription(&self) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .unregister_trade_subscription();
    }

    /// Register depth-of-market book subscription.
    pub fn register_book_subscription(&self, symbol: &str) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .register_book_subscription(symbol);
    }

    /// Unregister depth-of-market book subscription.
    pub fn unregister_book_subscription(&self, symbol: &str) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .unregister_book_subscription(symbol);
    }

    /// Register closed-bar subscription.
    pub fn register_bars_subscription(&self, symbol: &str, timeframe: Timeframe) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .register_bars_subscription(symbol, timeframe);
    }

    /// Unregister closed-bar subscription.
    pub fn unregister_bars_subscription(&self, symbol: &str, timeframe: Timeframe) {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .unregister_bars_subscription(symbol, timeframe);
    }

    /// Send a heartbeat ping to MT5 and return the server timestamp and negotiated protocol.
    pub fn ping(&self) -> Result<PingResult> {
        if let Some(fn_ping) = self.fn_ping {
            let mut server_time: i64 = 0;
            let mut protocol: u32 = 0;
            let ret = unsafe { fn_ping(&mut server_time, &mut protocol) };
            if ret == 1 {
                return Ok(PingResult {
                    server_time_secs: server_time,
                    protocol,
                });
            }
            return Err(Mt5Error::PipeDisconnected);
        }
        self.account_info().map(|_| PingResult {
            server_time_secs: chrono::Utc::now().timestamp(),
            protocol: crate::ffi::PROTOCOL_VERSION,
        })
    }

    /// Subscribe to connection disconnect notifications.
    #[cfg(feature = "async")]
    pub fn subscribe_disconnect(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.disconnect_tx.subscribe()
    }
}

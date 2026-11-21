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
    pub(crate) fn_positions: Option<FnPositions>,
    pub(crate) fn_orders: Option<FnOrders>,
    pub(crate) fn_deals: Option<FnDeals>,
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

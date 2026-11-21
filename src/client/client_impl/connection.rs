use crate::client::pipe::validate_pipe_name;
use crate::error::{Mt5Error, Result};
use crate::ffi::*;
#[cfg(feature = "async")]
use crate::types::*;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

use super::client::{ActiveGuard, Mt5Client, CLIENT_ACTIVE};

impl Mt5Client {
    /// Connect to MetaTrader 5 using the DLL path from the `MT5_DLL_PATH` environment variable,
    /// or `mt5_bridge.dll` adjacent to the running executable when the variable is unset.
    ///
    /// An absolute DLL path is always required: `MT5_DLL_PATH` must point at an existing file,
    /// and relative paths are canonicalized before loading (so they are never resolved through
    /// the OS ambient DLL search order). Paths inside world-writable directories are refused to
    /// avoid DLL preloading attacks.
    ///
    /// # Security Note
    /// In MetaTrader 5, the terminal session is already authenticated with the broker.
    /// The `password` parameter is matched by the EA against the bridge's shared `InpPipeSecret`
    /// token. You do NOT need to pass your live broker account password over the local IPC pipe;
    /// provide your configured `InpPipeSecret` token instead (or use [`connect_with_secret`](Self::connect_with_secret)).
    pub fn connect(login: i64, password: &str, server: &str) -> Result<Self> {
        let dll_path = std::env::var_os("MT5_DLL_PATH")
            .map(std::path::PathBuf::from)
            .or_else(Self::exe_adjacent_dll_path)
            .ok_or_else(|| Mt5Error::InvalidDllPath {
                path: String::new(),
                reason: "MT5_DLL_PATH is not set and no mt5_bridge.dll was found next to the \
                           executable; an absolute DLL path is required"
                    .to_string(),
            })?;
        Self::connect_with_dll(dll_path, login, password, server)
    }

    /// Connect to MetaTrader 5 using only a pipe authentication secret token (`InpPipeSecret`).
    ///
    /// This avoids passing broker account credentials (login/password/server) over the local IPC pipe,
    /// relying entirely on the application-level secret token configured on the Expert Advisor.
    pub fn connect_with_secret(secret: &str) -> Result<Self> {
        Self::connect(0, secret, "")
    }

    /// Connect to MetaTrader 5 using a custom DLL path and only a pipe authentication secret token.
    pub fn connect_with_secret_and_dll(dll_path: impl AsRef<Path>, secret: &str) -> Result<Self> {
        Self::connect_with_dll(dll_path, 0, secret, "")
    }

    /// Connect to MetaTrader 5 by loading the specified DLL path.
    pub fn connect_with_dll(
        dll_path: impl AsRef<Path>,
        login: i64,
        password: &str,
        server: &str,
    ) -> Result<Self> {
        if let Some(name) = std::env::var_os("MT5_PIPE_NAME") {
            validate_pipe_name(&name.to_string_lossy())?;
        }

        // Validate the DLL path before claiming the process-wide connection slot so a
        // rejected path never contends for the slot (and so a bad path fails fast
        // without blocking a concurrent connection).
        let path_ref = dll_path.as_ref();
        let canonical_path = Self::validate_dll_path(path_ref)?;

        // Verify the hash when configured, or require it with
        // MT5_DLL_REQUIRE_HASH=true.
        Self::verify_dll_hash(&canonical_path)?;

        if CLIENT_ACTIVE
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err(Mt5Error::AlreadyConnected);
        }

        // Any error from here until the connection is fully initialized must release
        // the process-wide slot; otherwise a partial DLL load would poison every
        // later connect attempt with `AlreadyConnected`.
        let mut active = ActiveGuard::arm();
        let path_str = canonical_path.to_string_lossy().to_string();

        let lib = match unsafe { libloading::Library::new(&canonical_path) } {
            Ok(l) => l,
            Err(source) => {
                return Err(Mt5Error::DllLoadError {
                    path: path_str.clone(),
                    source,
                });
            }
        };
        Self::verify_required_exports(&lib)?;

        let (
            fn_init,
            fn_shut,
            fn_rates,
            fn_acct,
            fn_send,
            fn_close,
            fn_close_magic,
            fn_close_deviation,
            fn_modify,
            fn_modify_magic,
            fn_sym_tick,
            fn_sym_info,
            fn_positions,
            fn_orders,
            fn_deals,
            fn_ping,
        ) = unsafe {
            let fn_init: FnInit =
                *lib.get(b"Initialize\0")
                    .map_err(|source| Mt5Error::SymbolNotFound {
                        symbol: "Initialize",
                        source,
                    })?;
            let fn_shut: FnShut =
                *lib.get(b"Shutdown\0")
                    .map_err(|source| Mt5Error::SymbolNotFound {
                        symbol: "Shutdown",
                        source,
                    })?;
            let fn_rates: FnRates =
                *lib.get(b"CopyRates\0")
                    .map_err(|source| Mt5Error::SymbolNotFound {
                        symbol: "CopyRates",
                        source,
                    })?;
            let fn_acct: FnAcct =
                *lib.get(b"AccountInfo\0")
                    .map_err(|source| Mt5Error::SymbolNotFound {
                        symbol: "AccountInfo",
                        source,
                    })?;
            let fn_send: FnSend =
                *lib.get(b"OrderSend\0")
                    .map_err(|source| Mt5Error::SymbolNotFound {
                        symbol: "OrderSend",
                        source,
                    })?;
            let fn_close: FnClose =
                *lib.get(b"OrderClose\0")
                    .map_err(|source| Mt5Error::SymbolNotFound {
                        symbol: "OrderClose",
                        source,
                    })?;

            let fn_modify: Option<FnModify> = lib
                .get::<FnModify>(b"OrderModify\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!("MT5 DLL: 'OrderModify' not found — SL/TP modification disabled")
                })
                .ok();

            let fn_close_magic: Option<FnCloseMagic> = lib
                .get::<FnCloseMagic>(b"OrderCloseWithMagic\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!(
                        "MT5 DLL: 'OrderCloseWithMagic' not found — closes fall back to plain \
                         OrderClose and may skip the magic ownership check"
                    )
                })
                .ok();

            let fn_close_deviation: Option<FnCloseDeviation> = lib
                .get::<FnCloseDeviation>(b"OrderCloseWithDeviation\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!(
                        "MT5 DLL: 'OrderCloseWithDeviation' not found — closes use the EA's \
                         default deviation instead of the configured slippage cap"
                    )
                })
                .ok();

            let fn_modify_magic: Option<FnModifyMagic> = lib
                .get::<FnModifyMagic>(b"OrderModifyWithMagic\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!(
                        "MT5 DLL: 'OrderModifyWithMagic' not found — SL/TP updates fall back to \
                         OrderModify and may skip the magic ownership check"
                    )
                })
                .ok();

            let fn_sym_tick: Option<FnSymTick> = lib
                .get::<FnSymTick>(b"SymbolInfoTick\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!("MT5 DLL: 'SymbolInfoTick' not found — real-time ticks disabled")
                })
                .ok();

            let fn_sym_info: Option<FnSymInfo> = lib
                .get::<FnSymInfo>(b"SymbolInfoFull\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!("MT5 DLL: 'SymbolInfoFull' not found — symbol specifications disabled")
                })
                .ok();

            let fn_positions: Option<FnPositions> = lib
                .get::<FnPositions>(b"PositionsGet\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!("MT5 DLL: 'PositionsGet' not found — positions querying disabled")
                })
                .ok();

            let fn_orders: Option<FnOrders> = lib
                .get::<FnOrders>(b"OrdersGet\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!("MT5 DLL: 'OrdersGet' not found — pending orders querying disabled")
                })
                .ok();

            let fn_deals: Option<FnDeals> = lib
                .get::<FnDeals>(b"DealsGet\0")
                .map(|s| *s)
                .map_err(|_| {
                    warn!("MT5 DLL: 'DealsGet' not found — deal-history reconciliation disabled")
                })
                .ok();

            let fn_ping: Option<FnPing> = lib.get::<FnPing>(b"Ping\0").map(|s| *s).ok();

            (
                fn_init,
                fn_shut,
                fn_rates,
                fn_acct,
                fn_send,
                fn_close,
                fn_close_magic,
                fn_close_deviation,
                fn_modify,
                fn_modify_magic,
                fn_sym_tick,
                fn_sym_info,
                fn_positions,
                fn_orders,
                fn_deals,
                fn_ping,
            )
        };

        #[cfg(feature = "async")]
        let (
            fn_subscribe_ticks,
            fn_unsubscribe_ticks,
            fn_subscribe_trade,
            fn_unsubscribe_trade,
            fn_subscribe_book,
            fn_unsubscribe_book,
            fn_subscribe_bars,
            fn_unsubscribe_bars,
            fn_poll_event,
            fn_events_dropped_total,
        ) = unsafe {
            let fn_subscribe_ticks: Option<FnSubscribeTicks> = lib
                .get::<FnSubscribeTicks>(b"SubscribeTicks\0")
                .map(|s| *s)
                .ok();

            let fn_unsubscribe_ticks: Option<FnUnsubscribeTicks> = lib
                .get::<FnUnsubscribeTicks>(b"UnsubscribeTicks\0")
                .map(|s| *s)
                .ok();

            let fn_subscribe_trade: Option<FnSubscribeTrade> = lib
                .get::<FnSubscribeTrade>(b"SubscribeTrade\0")
                .map(|s| *s)
                .ok();

            let fn_unsubscribe_trade: Option<FnUnsubscribeTrade> = lib
                .get::<FnUnsubscribeTrade>(b"UnsubscribeTrade\0")
                .map(|s| *s)
                .ok();

            let fn_subscribe_book: Option<FnSubscribeBook> = lib
                .get::<FnSubscribeBook>(b"SubscribeBook\0")
                .map(|s| *s)
                .ok();

            let fn_unsubscribe_book: Option<FnUnsubscribeBook> = lib
                .get::<FnUnsubscribeBook>(b"UnsubscribeBook\0")
                .map(|s| *s)
                .ok();

            let fn_subscribe_bars: Option<FnSubscribeBars> = lib
                .get::<FnSubscribeBars>(b"SubscribeBars\0")
                .map(|s| *s)
                .ok();

            let fn_unsubscribe_bars: Option<FnUnsubscribeBars> = lib
                .get::<FnUnsubscribeBars>(b"UnsubscribeBars\0")
                .map(|s| *s)
                .ok();

            let fn_poll_event: Option<FnPollEvent> =
                lib.get::<FnPollEvent>(b"PollEvent\0").map(|s| *s).ok();

            let fn_events_dropped_total: Option<FnEventsDroppedTotal> = lib
                .get::<FnEventsDroppedTotal>(b"EventsDroppedTotal\0")
                .map(|s| *s)
                .ok();

            (
                fn_subscribe_ticks,
                fn_unsubscribe_ticks,
                fn_subscribe_trade,
                fn_unsubscribe_trade,
                fn_subscribe_book,
                fn_unsubscribe_book,
                fn_subscribe_bars,
                fn_unsubscribe_bars,
                fn_poll_event,
                fn_events_dropped_total,
            )
        };

        #[cfg(feature = "async")]
        let event_bus = Arc::new(crate::stream::EventBus::new(2048));
        #[cfg(feature = "async")]
        let event_loop_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        #[cfg(feature = "async")]
        let (disconnect_tx, _) = tokio::sync::broadcast::channel(16);

        #[cfg(feature = "async")]
        let event_pump_handle = if let Some(poll_fn) = fn_poll_event {
            let running = Arc::clone(&event_loop_running);
            let bus = Arc::clone(&event_bus);
            let dis_tx = disconnect_tx.clone();
            std::thread::Builder::new()
                .name("mt5-event-pump".to_string())
                .spawn(move || {
                    let mut event_type: u16 = 0;
                    let mut buf = [0u8; 1024];
                    let mut out_len: u32 = 0;
                    while running.load(std::sync::atomic::Ordering::Relaxed) {
                        let res = unsafe {
                            poll_fn(
                                &mut event_type,
                                buf.as_mut_ptr(),
                                buf.len() as u32,
                                &mut out_len,
                                100,
                            )
                        };
                        if res < 0 {
                            let _ = dis_tx.send(());
                        } else if res == 1 && out_len > 0 {
                            match event_type {
                                1 if out_len >= std::mem::size_of::<Mt5TickEvent>() as u32 => {
                                    let raw: Mt5TickEvent = unsafe {
                                        std::ptr::read_unaligned(buf.as_ptr() as *const _)
                                    };
                                    bus.dispatch_tick(Tick::from_event(raw));
                                }
                                2 if out_len >= std::mem::size_of::<Mt5TradeEvent>() as u32 => {
                                    let raw: Mt5TradeEvent = unsafe {
                                        std::ptr::read_unaligned(buf.as_ptr() as *const _)
                                    };
                                    bus.dispatch_trade(TradeEvent::from_raw(raw));
                                }
                                3 if out_len >= std::mem::size_of::<Mt5BookEvent>() as u32 => {
                                    let raw: Mt5BookEvent = unsafe {
                                        std::ptr::read_unaligned(buf.as_ptr() as *const _)
                                    };
                                    bus.dispatch_book(BookEvent::from_raw(raw));
                                }
                                4 if out_len >= std::mem::size_of::<Mt5BarEvent>() as u32 => {
                                    let raw: Mt5BarEvent = unsafe {
                                        std::ptr::read_unaligned(buf.as_ptr() as *const _)
                                    };
                                    let (sym, tf, bar) = Bar::from_event(&raw);
                                    bus.dispatch_bar(&sym, tf, bar);
                                }
                                _ => {}
                            }
                        }
                    }
                })
                .ok()
        } else {
            None
        };

        let client = Mt5Client {
            _lib: lib,
            fn_init,
            fn_shut,
            fn_rates,
            fn_acct,
            fn_send,
            fn_close,
            fn_close_magic,
            fn_close_deviation,
            fn_modify,
            fn_modify_magic,
            fn_sym_tick,
            fn_sym_info,
            fn_positions,
            fn_orders,
            fn_deals,
            fn_ping,
            #[cfg(feature = "async")]
            fn_subscribe_ticks,
            #[cfg(feature = "async")]
            fn_unsubscribe_ticks,
            #[cfg(feature = "async")]
            fn_subscribe_trade,
            #[cfg(feature = "async")]
            fn_unsubscribe_trade,
            #[cfg(feature = "async")]
            fn_subscribe_book,
            #[cfg(feature = "async")]
            fn_unsubscribe_book,
            #[cfg(feature = "async")]
            fn_subscribe_bars,
            #[cfg(feature = "async")]
            fn_unsubscribe_bars,
            #[cfg(feature = "async")]
            fn_events_dropped_total,
            symbol_cache: Arc::new(Mutex::new(HashMap::new())),
            symbol_cache_ttl: super::cache::DEFAULT_SYMBOL_CACHE_TTL,
            #[cfg(feature = "async")]
            event_bus,
            #[cfg(feature = "async")]
            event_loop_running,
            #[cfg(feature = "async")]
            event_pump_handle,
            is_shutdown: std::sync::atomic::AtomicBool::new(false),
            session: Arc::new(Mutex::new(crate::session::Mt5Session::new(
                crate::session::SessionConfig::default(),
            ))),
            #[cfg(feature = "async")]
            disconnect_tx,
        };

        let pwd_c = CString::new(password)?;
        let srv_c = CString::new(server)?;

        info!(
            login = login,
            server = server,
            dll = %path_str,
            "Connecting to MetaTrader 5 bridge..."
        );

        let ret = unsafe { (client.fn_init)(login, pwd_c.as_ptr(), srv_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::InitFailed(ret));
        }

        info!(
            login = login,
            server = server,
            "MT5 bridge connected successfully"
        );
        active.disarm();
        Ok(client)
    }
}

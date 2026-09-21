use crate::error::{mt5_retcode_description, Mt5Error, Result};
use crate::ffi::*;
use crate::types::*;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_double, c_int};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// In-memory cache time-to-live for static symbol information.
pub const DEFAULT_SYMBOL_CACHE_TTL: Duration = Duration::from_secs(60);

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
    _lib: libloading::Library,
    fn_init: FnInit,
    fn_shut: FnShut,
    fn_rates: FnRates,
    fn_acct: FnAcct,
    fn_send: FnSend,
    fn_close: FnClose,
    fn_close_magic: Option<FnCloseMagic>,
    fn_modify: Option<FnModify>,
    fn_modify_magic: Option<FnModifyMagic>,
    fn_sym_tick: Option<FnSymTick>,
    fn_sym_info: Option<FnSymInfo>,
    fn_positions: Option<FnPositions>,
    fn_orders: Option<FnOrders>,
    fn_deals: Option<FnDeals>,
    fn_subscribe_ticks: Option<FnSubscribeTicks>,
    fn_unsubscribe_ticks: Option<FnUnsubscribeTicks>,
    fn_subscribe_trade: Option<FnSubscribeTrade>,
    fn_unsubscribe_trade: Option<FnUnsubscribeTrade>,
    fn_subscribe_book: Option<FnSubscribeBook>,
    fn_unsubscribe_book: Option<FnUnsubscribeBook>,
    #[allow(dead_code)]
    fn_poll_event: Option<FnPollEvent>,
    symbol_cache: Arc<Mutex<HashMap<String, (SymbolInfo, Instant)>>>,
    symbol_cache_ttl: Duration,
    #[cfg(feature = "async")]
    event_bus: Arc<crate::stream::EventBus>,
    #[cfg(feature = "async")]
    event_loop_running: Arc<std::sync::atomic::AtomicBool>,
}

// Safety: Mt5Client is safe to send and share across threads.
// All pipe I/O and state modifications are serialized via a Win32 CRITICAL_SECTION
// in the underlying C++ DLL. Note that because of this serialization, concurrent
// operations across threads (such as streaming ticks/bars and order execution) will
// serialize and contend on that single global mutex under the hood.
unsafe impl Send for Mt5Client {}
unsafe impl Sync for Mt5Client {}

impl Mt5Client {
    /// Resolve the default DLL path safely, preferring an explicit absolute path adjacent to
    /// the running executable or in the current working directory, avoiding ambient Windows DLL
    /// search order side-loading vulnerabilities.
    fn resolve_default_dll_path() -> std::path::PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let exe_dll = parent.join("mt5_bridge.dll");
                if exe_dll.is_file() {
                    return exe_dll;
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let cwd_dll = cwd.join("mt5_bridge.dll");
            if cwd_dll.is_file() {
                return cwd_dll;
            }
        }
        std::path::PathBuf::from("mt5_bridge.dll")
    }

    /// Connect to MetaTrader 5 using the DLL path from the `MT5_DLL_PATH` environment variable,
    /// or safely resolving `mt5_bridge.dll` adjacent to the executable / current working directory.
    ///
    /// # Security Note
    /// In MetaTrader 5, the terminal session is already authenticated with the broker.
    /// The `password` parameter is matched by the EA against the bridge's shared `InpPipeSecret`
    /// token. You do NOT need to pass your live broker account password over the IPC pipe;
    /// provide your configured `InpPipeSecret` token instead (or use [`connect_with_secret`](Self::connect_with_secret)).
    pub fn connect(login: i64, password: &str, server: &str) -> Result<Self> {
        let dll_path = std::env::var_os("MT5_DLL_PATH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(Self::resolve_default_dll_path);
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
        let path_ref = dll_path.as_ref();
        let canonical_path = if path_ref.is_relative() && path_ref.exists() {
            std::fs::canonicalize(path_ref).unwrap_or_else(|_| path_ref.to_path_buf())
        } else {
            path_ref.to_path_buf()
        };
        let path_str = canonical_path.to_string_lossy().to_string();

        let lib = unsafe { libloading::Library::new(&canonical_path) }.map_err(|source| {
            Mt5Error::DllLoadError {
                path: path_str.clone(),
                source,
            }
        })?;

        let (
            fn_init,
            fn_shut,
            fn_rates,
            fn_acct,
            fn_send,
            fn_close,
            fn_close_magic,
            fn_modify,
            fn_modify_magic,
            fn_sym_tick,
            fn_sym_info,
            fn_positions,
            fn_orders,
            fn_deals,
            fn_subscribe_ticks,
            fn_unsubscribe_ticks,
            fn_subscribe_trade,
            fn_unsubscribe_trade,
            fn_subscribe_book,
            fn_unsubscribe_book,
            fn_poll_event,
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
                .ok();

            let fn_modify_magic: Option<FnModifyMagic> = lib
                .get::<FnModifyMagic>(b"OrderModifyWithMagic\0")
                .map(|s| *s)
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

            let fn_poll_event: Option<FnPollEvent> = lib
                .get::<FnPollEvent>(b"PollEvent\0")
                .map(|s| *s)
                .ok();

            (
                fn_init,
                fn_shut,
                fn_rates,
                fn_acct,
                fn_send,
                fn_close,
                fn_close_magic,
                fn_modify,
                fn_modify_magic,
                fn_sym_tick,
                fn_sym_info,
                fn_positions,
                fn_orders,
                fn_deals,
                fn_subscribe_ticks,
                fn_unsubscribe_ticks,
                fn_subscribe_trade,
                fn_unsubscribe_trade,
                fn_subscribe_book,
                fn_unsubscribe_book,
                fn_poll_event,
            )
        };

        #[cfg(feature = "async")]
        let event_bus = Arc::new(crate::stream::EventBus::new(2048));
        #[cfg(feature = "async")]
        let event_loop_running = Arc::new(std::sync::atomic::AtomicBool::new(true));

        #[cfg(feature = "async")]
        if let Some(poll_fn) = fn_poll_event {
            let running = Arc::clone(&event_loop_running);
            let bus = Arc::clone(&event_bus);
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
                        if res == 1 && out_len > 0 {
                            match event_type {
                                1 if out_len >= std::mem::size_of::<Mt5TickEvent>() as u32 => {
                                    let raw: Mt5TickEvent = unsafe { std::ptr::read(buf.as_ptr() as *const _) };
                                    bus.dispatch_tick(Tick::from_event(raw));
                                }
                                2 if out_len >= std::mem::size_of::<Mt5TradeEvent>() as u32 => {
                                    let raw: Mt5TradeEvent = unsafe { std::ptr::read(buf.as_ptr() as *const _) };
                                    bus.dispatch_trade(TradeEvent::from_raw(raw));
                                }
                                3 if out_len >= std::mem::size_of::<Mt5BookEvent>() as u32 => {
                                    let raw: Mt5BookEvent = unsafe { std::ptr::read(buf.as_ptr() as *const _) };
                                    bus.dispatch_book(BookEvent::from_raw(raw));
                                }
                                _ => {}
                            }
                        }
                    }
                })
                .ok();
        }

        let client = Mt5Client {
            _lib: lib,
            fn_init,
            fn_shut,
            fn_rates,
            fn_acct,
            fn_send,
            fn_close,
            fn_close_magic,
            fn_modify,
            fn_modify_magic,
            fn_sym_tick,
            fn_sym_info,
            fn_positions,
            fn_orders,
            fn_deals,
            fn_subscribe_ticks,
            fn_unsubscribe_ticks,
            fn_subscribe_trade,
            fn_unsubscribe_trade,
            fn_subscribe_book,
            fn_unsubscribe_book,
            fn_poll_event,
            symbol_cache: Arc::new(Mutex::new(HashMap::new())),
            symbol_cache_ttl: DEFAULT_SYMBOL_CACHE_TTL,
            #[cfg(feature = "async")]
            event_bus,
            #[cfg(feature = "async")]
            event_loop_running,
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
        Ok(client)
    }

    /// Set a custom cache TTL for symbol specifications.
    pub fn set_symbol_cache_ttl(&mut self, ttl: Duration) {
        self.symbol_cache_ttl = ttl;
    }

    /// Query current account balance, equity, and margin information.
    pub fn account_info(&self) -> Result<AccountInfo> {
        let (mut balance, mut equity, mut margin, mut free_margin) = (0.0, 0.0, 0.0, 0.0);
        let ret =
            unsafe { (self.fn_acct)(&mut balance, &mut equity, &mut margin, &mut free_margin) };

        if ret != 1 {
            return Err(Mt5Error::AccountInfoFailed(ret));
        }

        Ok(AccountInfo {
            balance,
            equity,
            margin,
            free_margin,
        })
    }

    /// Query symbol specifications (point size, tick value, lot sizes, spread, digits).
    ///
    /// Results are cached locally in-memory for `symbol_cache_ttl` duration.
    pub fn symbol_info(&self, symbol: &str) -> Result<SymbolInfo> {
        let upper = symbol.to_uppercase();

        if let Some((info, timestamp)) = self.symbol_cache.lock().unwrap().get(&upper).cloned() {
            if timestamp.elapsed() < self.symbol_cache_ttl {
                return Ok(info);
            }
        }

        let fn_sym = self
            .fn_sym_info
            .ok_or(Mt5Error::UnsupportedFeature("SymbolInfoFull"))?;

        let sym_c = CString::new(symbol)?;
        let mut raw = Mt5SymInfo::default();

        let ret = unsafe { fn_sym(sym_c.as_ptr(), &mut raw) };
        if ret != 1 {
            return Err(Mt5Error::SymbolInfoFailed(symbol.to_string()));
        }

        let info = SymbolInfo::from_raw(symbol, raw);
        self.symbol_cache
            .lock()
            .unwrap()
            .insert(upper, (info.clone(), Instant::now()));

        Ok(info)
    }

    /// Query symbol specifications directly from MT5, bypassing the in-memory cache,
    /// and update the cache with fresh values.
    pub fn symbol_info_fresh(&self, symbol: &str) -> Result<SymbolInfo> {
        let upper = symbol.to_uppercase();
        let fn_sym = self
            .fn_sym_info
            .ok_or(Mt5Error::UnsupportedFeature("SymbolInfoFull"))?;

        let sym_c = CString::new(symbol)?;
        let mut raw = Mt5SymInfo::default();

        let ret = unsafe { fn_sym(sym_c.as_ptr(), &mut raw) };
        if ret != 1 {
            return Err(Mt5Error::SymbolInfoFailed(symbol.to_string()));
        }

        let info = SymbolInfo::from_raw(symbol, raw);
        self.symbol_cache
            .lock()
            .unwrap()
            .insert(upper, (info.clone(), Instant::now()));

        Ok(info)
    }

    /// Clear all cached symbol specifications.
    pub fn clear_symbol_cache(&self) {
        self.symbol_cache.lock().unwrap().clear();
    }

    /// Query the latest price tick for a symbol.
    pub fn symbol_tick(&self, symbol: &str) -> Result<Tick> {
        let fn_sym_tick = self
            .fn_sym_tick
            .ok_or(Mt5Error::UnsupportedFeature("SymbolInfoTick"))?;

        let sym_c = CString::new(symbol)?;
        let mut raw = Mt5Tick::default();

        let ret = unsafe { fn_sym_tick(sym_c.as_ptr(), &mut raw) };
        if ret != 1 {
            return Err(Mt5Error::SymbolTickFailed(symbol.to_string()));
        }

        Ok(Tick::from_raw(symbol, raw))
    }

    /// Fetch historical rates (OHLCV bars) within a UTC timestamp range `[from, to]`.
    pub fn copy_rates(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Rate>> {
        if from > to {
            return Err(Mt5Error::InvalidTimeRange {
                start: from,
                end: to,
            });
        }

        let sym_c = CString::new(symbol)?;
        let tf_const = timeframe.to_mt5_const();
        let tf_secs = timeframe.seconds().max(1);

        // Maximum limit to prevent unbounded memory allocation in a single unchunked call
        const MAX_SINGLE_FETCH_BARS: i64 = 1_000_000;
        let diff_bars = (to - from) / tf_secs;
        if diff_bars > MAX_SINGLE_FETCH_BARS {
            return Err(Mt5Error::Other(format!(
                "Requested range ({} bars) exceeds maximum single fetch limit of {} bars. Please use copy_rates_chunked() for large historical datasets.",
                diff_bars, MAX_SINGLE_FETCH_BARS
            )));
        }

        let estimated_bars = ((to - from) / tf_secs + 100).max(100) as usize;
        let mut buf = vec![Mt5Rate::default(); estimated_bars];

        let filled = unsafe {
            (self.fn_rates)(
                sym_c.as_ptr(),
                tf_const,
                from,
                to,
                buf.as_mut_ptr(),
                buf.len() as c_int,
            )
        };

        if filled < 0 {
            return Err(Mt5Error::CopyRatesFailed {
                symbol: symbol.to_string(),
                status: filled,
            });
        }

        let n = (filled as usize).min(buf.len());
        let rates = buf[..n].iter().copied().map(Rate::from).collect();
        Ok(rates)
    }

    /// Robust, chunked historical data downloader with completeness and missing range tracking.
    pub fn copy_rates_chunked_detailed(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start: i64,
        end: i64,
        chunk_bars: usize,
    ) -> Result<HistoryResult> {
        if start > end {
            return Err(Mt5Error::InvalidTimeRange { start, end });
        }

        let sym_c = CString::new(symbol)?;
        let tf_const = timeframe.to_mt5_const();
        let tf_secs = timeframe.seconds().max(1);

        let chunk_size = chunk_bars.max(100);
        let chunk_duration = (chunk_size as i64) * tf_secs;

        let mut all_rates: Vec<Rate> = Vec::new();
        let mut missing_ranges: Vec<(i64, i64)> = Vec::new();
        let mut current_start = start;

        while current_start <= end {
            let current_end = (current_start + chunk_duration).min(end);
            let max_bars = ((current_end - current_start) / tf_secs + 20) as usize;

            let mut attempts = 0;
            let max_attempts = 30; // up to 3 seconds wait
            let mut last_count = -1;
            let mut chunk_result: Vec<Rate> = Vec::new();
            let mut chunk_failed = false;

            while attempts < max_attempts {
                let mut buf = vec![Mt5Rate::default(); max_bars];
                let filled = unsafe {
                    (self.fn_rates)(
                        sym_c.as_ptr(),
                        tf_const,
                        current_start,
                        current_end,
                        buf.as_mut_ptr(),
                        buf.len() as c_int,
                    )
                };

                if filled > 0 {
                    if filled > last_count {
                        last_count = filled;
                        chunk_result = buf[..filled as usize]
                            .iter()
                            .copied()
                            .map(Rate::from)
                            .collect();
                    } else {
                        // Rate count stabilized
                        break;
                    }
                } else if filled == 0 {
                    last_count = 0;
                    chunk_result.clear();
                    if attempts >= 5 {
                        break;
                    }
                } else {
                    // status < 0 (server still loading or error)
                    if attempts >= 10 {
                        warn!(
                            symbol = symbol,
                            start = current_start,
                            "CopyRates returned error multiple times; tracking as missing range"
                        );
                        chunk_failed = true;
                        break;
                    }
                }

                attempts += 1;
                std::thread::sleep(Duration::from_millis(100));
            }

            if chunk_failed {
                missing_ranges.push((current_start, current_end));
            }

            let filtered_chunk: Vec<Rate> = chunk_result
                .into_iter()
                .filter(|r| r.time >= current_start && r.time <= current_end)
                .collect();

            all_rates.extend(filtered_chunk);
            current_start = current_end + 1;
        }

        // Sort and deduplicate by timestamp
        all_rates.sort_by_key(|r| r.time);
        all_rates.dedup_by_key(|r| r.time);

        let complete = missing_ranges.is_empty();
        Ok(HistoryResult {
            rates: all_rates,
            complete,
            missing_ranges,
        })
    }

    /// Robust, chunked historical data downloader.
    ///
    /// Requests historical bars in chunks (default 2,000 bars per chunk) with a retry loop
    /// to give MetaTrader 5 time to asynchronously fetch older history from the broker server.
    pub fn copy_rates_chunked(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start: i64,
        end: i64,
        chunk_bars: usize,
    ) -> Result<Vec<Rate>> {
        let res = self.copy_rates_chunked_detailed(symbol, timeframe, start, end, chunk_bars)?;
        if !res.complete {
            warn!(
                symbol = symbol,
                missing = res.missing_ranges.len(),
                "copy_rates_chunked finished with missing ranges"
            );
        }
        Ok(res.rates)
    }

    /// Convenience wrapper around `copy_rates` that returns clean `Bar` structures.
    pub fn copy_bars(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Bar>> {
        let rates = self.copy_rates(symbol, timeframe, from, to)?;
        Ok(rates.into_iter().map(Bar::from).collect())
    }

    /// Send a trading order (Market Buy/Sell or Limit/Stop orders).
    pub fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        req.validate()?;

        let sym_c = CString::new(req.symbol.as_str())?;
        let effective_cmt = req.effective_comment();
        let cmt_c = CString::new(effective_cmt.as_str())?;
        let otype = req.order_type as c_int;
        let dev = req.deviation.unwrap_or(10);
        let exp = req.expiration.unwrap_or(0);
        let mag = req.magic.unwrap_or(0);

        let mut res = Mt5TradeResult::default();
        let ret = unsafe {
            (self.fn_send)(
                sym_c.as_ptr(),
                otype,
                req.volume as c_double,
                req.price as c_double,
                req.stop_loss as c_double,
                req.take_profit as c_double,
                cmt_c.as_ptr(),
                dev,
                exp,
                mag,
                &mut res,
            )
        };

        if ret == MT5_ERR_UNKNOWN_EXECUTION {
            return Err(Mt5Error::UnknownExecutionState {
                symbol: req.symbol.clone(),
                client_order_id: req.client_order_id.clone(),
                description: "Order request sent, but pipe response was lost or timed out. Reconcile broker state before retrying.".to_string(),
            });
        }

        if ret == MT5_ERR_SEND_FAILED {
            return Err(Mt5Error::TransmissionFailed(format!(
                "Failed to transmit OrderSend packet to MT5 bridge for symbol {}",
                req.symbol
            )));
        }

        if ret == MT5_ERR_PIPE_DISCONNECTED {
            return Err(Mt5Error::TransmissionFailed(
                "Bridge pipe is disconnected".to_string(),
            ));
        }

        let trade_result = TradeResult::from_raw(res);

        if ret != 1 || !trade_result.is_success() {
            return Err(Mt5Error::OrderSendFailed {
                symbol: req.symbol.clone(),
                retcode: trade_result.retcode,
                description: mt5_retcode_description(trade_result.retcode),
            });
        }

        debug!(
            symbol = %req.symbol,
            order_type = ?req.order_type,
            volume = req.volume,
            ticket = trade_result.order,
            deal = trade_result.deal,
            price = trade_result.price,
            status = ?trade_result.status(),
            client_order_id = ?req.client_order_id,
            "Order executed successfully"
        );

        Ok(trade_result)
    }

    /// Close an existing position by its ticket number.
    pub fn order_close(&self, ticket: u64) -> Result<TradeResult> {
        self.order_close_with_magic(ticket, 0)
    }

    /// Close an existing position by ticket number, verifying that it belongs to the given magic number.
    pub fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        if ticket == 0 {
            return Err(Mt5Error::Other("Order ticket cannot be 0".to_string()));
        }

        let mut res = Mt5TradeResult::default();
        let ret = if let Some(fn_close_mag) = self.fn_close_magic {
            unsafe { fn_close_mag(ticket, magic, &mut res) }
        } else {
            unsafe { (self.fn_close)(ticket, &mut res) }
        };

        if ret == MT5_ERR_UNKNOWN_EXECUTION {
            return Err(Mt5Error::UnknownExecutionState {
                symbol: String::new(),
                client_order_id: None,
                description: format!("OrderClose sent for ticket {ticket}, but pipe response was lost. Reconcile broker state before retrying."),
            });
        }

        if ret == MT5_ERR_SEND_FAILED {
            return Err(Mt5Error::TransmissionFailed(format!(
                "Failed to transmit OrderClose packet for ticket {ticket}"
            )));
        }

        let trade_result = TradeResult::from_raw(res);

        if ret != 1 || !trade_result.is_success() {
            return Err(Mt5Error::OrderCloseFailed {
                ticket,
                retcode: trade_result.retcode,
                description: mt5_retcode_description(trade_result.retcode),
            });
        }

        debug!(
            ticket = ticket,
            deal = trade_result.deal,
            price = trade_result.price,
            status = ?trade_result.status(),
            "Position closed successfully"
        );

        Ok(trade_result)
    }

    /// Cancel an active pending order (Buy/Sell Limit or Buy/Sell Stop) by its ticket number.
    ///
    /// This is an ergonomic alias for [`order_close`](Self::order_close) specifically intended
    /// for pending orders. Under the hood, the EA detects that the ticket represents a pending order
    /// and issues an MT5 `TRADE_ACTION_REMOVE` order request.
    pub fn order_cancel(&self, ticket: u64) -> Result<TradeResult> {
        self.order_close(ticket)
    }

    /// Modify the Stop Loss and/or Take Profit of an open position or pending order.
    pub fn order_modify(
        &self,
        ticket: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        self.order_modify_with_magic(ticket, 0, stop_loss, take_profit)
    }

    /// Modify the Stop Loss and/or Take Profit of an open position or pending order,
    /// verifying that it belongs to the given magic number.
    pub fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        if ticket == 0 {
            return Err(Mt5Error::Other("Order ticket cannot be 0".to_string()));
        }
        if stop_loss < 0.0 || stop_loss.is_nan() || stop_loss.is_infinite() {
            return Err(Mt5Error::Other(format!(
                "Invalid stop loss {}: stop loss cannot be negative or NaN",
                stop_loss
            )));
        }
        if take_profit < 0.0 || take_profit.is_nan() || take_profit.is_infinite() {
            return Err(Mt5Error::Other(format!(
                "Invalid take profit {}: take profit cannot be negative or NaN",
                take_profit
            )));
        }

        let mut res = Mt5TradeResult::default();
        let ret = if let Some(fn_mod_mag) = self.fn_modify_magic {
            unsafe {
                fn_mod_mag(
                    ticket,
                    magic,
                    stop_loss as c_double,
                    take_profit as c_double,
                    &mut res,
                )
            }
        } else {
            let fn_mod = self
                .fn_modify
                .ok_or(Mt5Error::UnsupportedFeature("OrderModify"))?;
            unsafe {
                fn_mod(
                    ticket,
                    stop_loss as c_double,
                    take_profit as c_double,
                    &mut res,
                )
            }
        };

        if ret == MT5_ERR_UNKNOWN_EXECUTION {
            return Err(Mt5Error::UnknownExecutionState {
                symbol: String::new(),
                client_order_id: None,
                description: format!("OrderModify sent for ticket {ticket}, but pipe response was lost. Reconcile broker state before retrying."),
            });
        }

        if ret == MT5_ERR_SEND_FAILED {
            return Err(Mt5Error::TransmissionFailed(format!(
                "Failed to transmit OrderModify packet for ticket {ticket}"
            )));
        }

        let trade_result = TradeResult::from_raw(res);

        if ret != 1 || !trade_result.is_success() {
            return Err(Mt5Error::OrderModifyFailed {
                ticket,
                retcode: trade_result.retcode,
                description: mt5_retcode_description(trade_result.retcode),
            });
        }

        debug!(
            ticket = ticket,
            sl = stop_loss,
            tp = take_profit,
            retcode = trade_result.retcode,
            "Order modified successfully"
        );
        Ok(trade_result)
    }

    /// Query all active open positions in MetaTrader 5.
    pub fn positions(&self) -> Result<Vec<Position>> {
        self.positions_filtered(None, None)
    }

    /// Query open positions filtered by magic number and/or symbol.
    pub fn positions_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        let fn_pos = self
            .fn_positions
            .ok_or(Mt5Error::UnsupportedFeature("PositionsGet"))?;

        let mag = magic.unwrap_or(0);
        let sym_c = match symbol {
            Some(s) => Some(CString::new(s)?),
            None => None,
        };
        let sym_ptr = sym_c
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null());

        const CAPACITY: usize = 512;
        let mut buf = vec![Mt5Position::default(); CAPACITY];

        let count = unsafe { fn_pos(buf.as_mut_ptr(), CAPACITY as c_int, mag, sym_ptr) };
        if count < 0 {
            return Err(Mt5Error::PositionsFailed(count));
        }

        let n = (count as usize).min(buf.len());
        let positions = buf[..n].iter().copied().map(Position::from_raw).collect();
        Ok(positions)
    }

    /// Query all active working pending orders in MetaTrader 5.
    pub fn pending_orders(&self) -> Result<Vec<WorkingOrder>> {
        self.pending_orders_filtered(None, None)
    }

    /// Query active working pending orders filtered by magic number and/or symbol.
    pub fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        let fn_ord = self
            .fn_orders
            .ok_or(Mt5Error::UnsupportedFeature("OrdersGet"))?;

        let mag = magic.unwrap_or(0);
        let sym_c = match symbol {
            Some(s) => Some(CString::new(s)?),
            None => None,
        };
        let sym_ptr = sym_c
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null());

        const CAPACITY: usize = 512;
        let mut buf = vec![Mt5Order::default(); CAPACITY];

        let count = unsafe { fn_ord(buf.as_mut_ptr(), CAPACITY as c_int, mag, sym_ptr) };
        if count < 0 {
            return Err(Mt5Error::OrdersFailed(count));
        }

        let n = (count as usize).min(buf.len());
        let orders = buf[..n].iter().copied().map(WorkingOrder::from_raw).collect();
        Ok(orders)
    }

    /// Maximum number of deals fetched by a single [`deals_filtered`](Self::deals_filtered) call.
    pub const DEALS_CAPACITY: usize = 4096;

    /// Query all completed trade deals (MT5 trade history) within `from <= time <= to`, in UTC seconds.
    pub fn deals(&self, from: i64, to: i64) -> Result<Vec<Deal>> {
        self.deals_filtered(from, to, None, None)
    }

    /// Query completed trade deals (MT5 trade history) with `from <= time <= to`, in UTC seconds,
    /// optionally filtered by magic number and/or symbol.
    ///
    /// Deal history is the authoritative record of what actually executed. It lets
    /// reconciliation reconstruct fill-by-fill volume, average price and deal tickets
    /// independently of whether a synchronous [`TradeResult`] ever reached the caller, and it
    /// still shows fills whose position has since been closed.
    ///
    /// If the result would exceed [`DEALS_CAPACITY`](Self::DEALS_CAPACITY) an error is returned
    /// instead of a silently truncated list: an incomplete history could make an executed
    /// order look absent. Narrow the time window or filters and retry.
    pub fn deals_filtered(
        &self,
        from: i64,
        to: i64,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        if from > to {
            return Err(Mt5Error::InvalidTimeRange {
                start: from,
                end: to,
            });
        }
        let fn_deals = self
            .fn_deals
            .ok_or(Mt5Error::UnsupportedFeature("DealsGet"))?;

        let mag = magic.unwrap_or(0);
        let sym_c = match symbol {
            Some(s) => Some(CString::new(s)?),
            None => None,
        };
        let sym_ptr = sym_c
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null());

        let mut buf = vec![Mt5Deal::default(); Self::DEALS_CAPACITY];
        let count = unsafe {
            fn_deals(
                buf.as_mut_ptr(),
                Self::DEALS_CAPACITY as c_int,
                from,
                to,
                mag,
                sym_ptr,
            )
        };
        if count < 0 {
            return Err(Mt5Error::DealsFailed(count));
        }
        let n = (count as usize).min(buf.len());
        if n >= Self::DEALS_CAPACITY {
            return Err(Mt5Error::ReconciliationError(format!(
                "deal history for the requested window reached the {} deal buffer limit and may be truncated; narrow the window or filters",
                Self::DEALS_CAPACITY
            )));
        }
        Ok(buf[..n].iter().copied().map(Deal::from_raw).collect())
    }

    /// Subscribe to real-time pushed market data ticks for a symbol (protocol v5+ push model).
    ///
    /// By default uses [`StreamMode::Latest`] which drops stale quotes on consumer lag to prioritize lowest latency.
    #[cfg(feature = "async")]
    pub fn subscribe_ticks(&self, symbol: &str) -> Result<crate::stream::TickSubscription> {
        self.subscribe_ticks_with_mode(symbol, StreamMode::Latest)
    }

    /// Subscribe to real-time pushed market data ticks for a symbol with an explicit [`StreamMode`] (protocol v5+).
    #[cfg(feature = "async")]
    pub fn subscribe_ticks_with_mode(
        &self,
        symbol: &str,
        mode: StreamMode,
    ) -> Result<crate::stream::TickSubscription> {
        let fn_sub = self
            .fn_subscribe_ticks
            .ok_or(Mt5Error::UnsupportedFeature("SubscribeTicks"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_sub(sym_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "SubscribeTicks failed for {} (status={})",
                symbol, ret
            )));
        }
        let rx = self.event_bus.subscribe_ticks(symbol.trim());
        Ok(crate::stream::TickSubscription::new(symbol.trim(), mode, rx))
    }

    /// Unsubscribe from real-time pushed ticks for a symbol (protocol v5+).
    #[cfg(feature = "async")]
    pub fn unsubscribe_ticks(&self, symbol: &str) -> Result<()> {
        let fn_unsub = self
            .fn_unsubscribe_ticks
            .ok_or(Mt5Error::UnsupportedFeature("UnsubscribeTicks"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_unsub(sym_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "UnsubscribeTicks failed for {} (status={})",
                symbol, ret
            )));
        }
        Ok(())
    }

    /// Subscribe to asynchronous broker trade transaction events (protocol v5+).
    #[cfg(feature = "async")]
    pub fn subscribe_trade(&self) -> Result<tokio::sync::broadcast::Receiver<TradeEvent>> {
        let fn_sub = self
            .fn_subscribe_trade
            .ok_or(Mt5Error::UnsupportedFeature("SubscribeTrade"))?;
        let ret = unsafe { fn_sub() };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "SubscribeTrade failed (status={})",
                ret
            )));
        }
        Ok(self.event_bus.subscribe_trade())
    }

    /// Unsubscribe from asynchronous broker trade transaction events (protocol v5+).
    #[cfg(feature = "async")]
    pub fn unsubscribe_trade(&self) -> Result<()> {
        let fn_unsub = self
            .fn_unsubscribe_trade
            .ok_or(Mt5Error::UnsupportedFeature("UnsubscribeTrade"))?;
        let ret = unsafe { fn_unsub() };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "UnsubscribeTrade failed (status={})",
                ret
            )));
        }
        Ok(())
    }

    /// Subscribe to depth-of-market book events for a symbol (protocol v5+).
    #[cfg(feature = "async")]
    pub fn subscribe_book(&self, symbol: &str) -> Result<tokio::sync::broadcast::Receiver<BookEvent>> {
        let fn_sub = self
            .fn_subscribe_book
            .ok_or(Mt5Error::UnsupportedFeature("SubscribeBook"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_sub(sym_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "SubscribeBook failed for {} (status={})",
                symbol, ret
            )));
        }
        Ok(self.event_bus.subscribe_book(symbol.trim()))
    }

    /// Unsubscribe from depth-of-market book events for a symbol (protocol v5+).
    #[cfg(feature = "async")]
    pub fn unsubscribe_book(&self, symbol: &str) -> Result<()> {
        let fn_unsub = self
            .fn_unsubscribe_book
            .ok_or(Mt5Error::UnsupportedFeature("UnsubscribeBook"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_unsub(sym_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "UnsubscribeBook failed for {} (status={})",
                symbol, ret
            )));
        }
        Ok(())
    }

    /// Access the underlying [`EventBus`](crate::stream::EventBus) directly.
    #[cfg(feature = "async")]
    pub fn event_bus(&self) -> Arc<crate::stream::EventBus> {
        Arc::clone(&self.event_bus)
    }

    /// Gracefully shutdown the named pipe connection to MetaTrader 5.
    pub fn shutdown(&self) -> Result<()> {
        #[cfg(feature = "async")]
        self.event_loop_running.store(false, std::sync::atomic::Ordering::SeqCst);
        let ret = unsafe { (self.fn_shut)() };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "Shutdown returned non-1 status code: {}",
                ret
            )));
        }
        Ok(())
    }
}

impl Drop for Mt5Client {
    fn drop(&mut self) {
        #[cfg(feature = "async")]
        self.event_loop_running.store(false, std::sync::atomic::Ordering::SeqCst);
        // Shutdown is intentionally idempotent in the DLL (returns 1 if already disconnected).
        unsafe {
            let _ = (self.fn_shut)();
        }
    }
}

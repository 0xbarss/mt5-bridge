use crate::error::{Mt5Error, Result};
use crate::ffi::*;
use crate::market::calendar::parse_calendar_events;
use crate::types::*;
use crate::CalendarEvent;
use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::ptr;

use super::client::Mt5Client;

impl Mt5Client {
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
        let sym_ptr = sym_c.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());

        const CAPACITY: usize = 4096;
        let mut buf = vec![Mt5Position::default(); CAPACITY];

        let count = unsafe { fn_pos(buf.as_mut_ptr(), CAPACITY as c_int, mag, sym_ptr) };
        if count < 0 {
            return Err(Mt5Error::PositionsFailed(count));
        }
        if count as usize > CAPACITY {
            return Err(Mt5Error::PositionsTruncated {
                returned: CAPACITY,
                total: count as usize,
            });
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
        let sym_ptr = sym_c.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());

        const CAPACITY: usize = 4096;
        let mut buf = vec![Mt5Order::default(); CAPACITY];

        let count = unsafe { fn_ord(buf.as_mut_ptr(), CAPACITY as c_int, mag, sym_ptr) };
        if count < 0 {
            return Err(Mt5Error::OrdersFailed(count));
        }
        if count as usize > CAPACITY {
            return Err(Mt5Error::OrdersTruncated {
                returned: CAPACITY,
                total: count as usize,
            });
        }

        let n = (count as usize).min(buf.len());
        let orders = buf[..n]
            .iter()
            .copied()
            .map(WorkingOrder::from_raw)
            .collect();
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
        let sym_ptr = sym_c.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());

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
        if count as usize > Self::DEALS_CAPACITY {
            return Err(Mt5Error::DealHistoryTruncated {
                returned: Self::DEALS_CAPACITY,
                capacity: Self::DEALS_CAPACITY,
            });
        }
        let n = (count as usize).min(buf.len());
        Ok(buf[..n].iter().copied().map(Deal::from_raw).collect())
    }

    /// Maximum number of symbols returned by a single [`symbols`](Self::symbols) call.
    pub const SYMBOLS_PAGE_SIZE: u32 = 1000;

    /// Maximum symbol name length in bytes (MT5 symbol names fit in a 32-byte fixed field).
    pub const SYMBOL_NAME_MAX_BYTES: usize = 32;

    /// Query the total number of symbols available in MT5 Market Watch.
    pub fn symbols_total(&self) -> Result<u32> {
        let fn_symbols_total = self
            .fn_symbols_total
            .ok_or(Mt5Error::UnsupportedFeature("SymbolsTotal"))?;

        let count = unsafe { fn_symbols_total() };
        if count < 0 {
            return Err(Mt5Error::SymbolsTotalFailed(count));
        }
        Ok(count as u32)
    }

    /// Query a page of symbols from MT5 Market Watch.
    ///
    /// Returns up to `page_size` [`SymbolInfo`] entries starting at `page * page_size`.
    /// Use [`symbols`](Self::symbols) to fetch all symbols in one call (paging internally).
    pub fn symbols_page(&self, page: u32, page_size: u32) -> Result<Vec<SymbolInfo>> {
        let fn_symbols_get = self
            .fn_symbols_get
            .ok_or(Mt5Error::UnsupportedFeature("SymbolsGet"))?;

        let page_size = page_size.max(1);
        let entry_size = 4 + Self::SYMBOL_NAME_MAX_BYTES + std::mem::size_of::<Mt5SymInfo>();
        let capacity = (page_size as usize + 1) * entry_size;
        let mut buf = vec![0u8; capacity];

        let total = unsafe {
            fn_symbols_get(
                page,
                page_size,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as c_int,
            )
        };
        if total < 0 {
            return Err(Mt5Error::SymbolsGetFailed(total));
        }

        let mut symbols = Vec::new();
        let mut off = 0;
        while off + 4 <= buf.len() {
            let name_len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
            let entry_size = 4 + name_len as usize + std::mem::size_of::<Mt5SymInfo>();
            if off + entry_size > buf.len() {
                break;
            }
            let name_bytes = &buf[off + 4..off + 4 + name_len as usize];
            let name = String::from_utf8_lossy(name_bytes).to_string();
            let info_raw = unsafe {
                ptr::read_unaligned(buf[off + 4 + name_len as usize..].as_ptr() as *const Mt5SymInfo)
            };
            symbols.push(SymbolInfo::from_raw(&name, info_raw));
            off += entry_size;
        }
        Ok(symbols)
    }

    /// Query all symbols available in MT5 Market Watch.
    ///
    /// Pages through [`symbols_page`](Self::symbols_page) internally using [`SYMBOLS_PAGE_SIZE`](Self::SYMBOLS_PAGE_SIZE).
    /// On any page error, returns the symbols collected so far along with the error.
    pub fn symbols(&self) -> Result<Vec<SymbolInfo>> {
        let mut all = Vec::new();
        let mut page = 0u32;
        loop {
            let page_syms = self.symbols_page(page, Self::SYMBOLS_PAGE_SIZE)?;
            let page_len = page_syms.len();
            all.extend(page_syms);
            if page_len < Self::SYMBOLS_PAGE_SIZE as usize {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    /// Query symbols filtered by predicate `f`.
    pub fn symbols_filtered(&self, f: impl Fn(&SymbolInfo) -> bool) -> Result<Vec<SymbolInfo>> {
        let all = self.symbols()?;
        Ok(all.into_iter().filter(|s| f(s)).collect())
    }

    /// Select or deselect a symbol in MT5 Market Watch.
    ///
    /// When `enable` is true, the symbol is added to Market Watch (required before
    /// many MT5 operations such as `copy_bars` or `symbol_info`).
    pub fn symbol_select(&self, symbol: &str, enable: bool) -> Result<()> {
        let fn_symbol_select = self
            .fn_symbol_select
            .ok_or(Mt5Error::UnsupportedFeature("SymbolSelect"))?;

        let sym_c = CString::new(symbol)?;
        let ret = unsafe { fn_symbol_select(sym_c.as_ptr(), enable as c_int) };
        if ret != 1 {
            return Err(Mt5Error::SymbolSelectFailed(symbol.to_string()));
        }
        Ok(())
    }

    /// Query economic calendar events for the UTC half-open interval `[from, to)`.
    ///
    /// Returns all events in the window. The FFI status carries the total event
    /// count; the packed records are decoded from the buffer. An `id == 0`
    /// record marks the end of the stream (the buffer is zero-initialised and
    /// MT5 calendar ids are positive), so no trailing garbage is decoded.
    pub fn calendar_events(&self, from: u64, to: u64) -> Result<Vec<CalendarEvent>> {
        let fn_calendar_events = self
            .fn_calendar_events
            .ok_or(Mt5Error::UnsupportedFeature("CalendarEvents"))?;

        let capacity = 16 * 1024 * 1024;
        let mut buf = vec![0u8; capacity];

        let total = unsafe {
            fn_calendar_events(
                from,
                to,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as c_int,
            )
        };
        if total < 0 {
            return Err(Mt5Error::CalendarEventsFailed(total));
        }

        parse_calendar_events(&buf).map_err(Mt5Error::Other)
    }

    /// Query the countries that have economic calendar events.
    ///
    /// Returns country names (e.g. `"US"`, `"EU"`, `"JP"`) suitable for pairing
    /// with [`NewsFilter::currencies`] gating.
    pub fn calendar_countries(&self) -> Result<Vec<String>> {
        let fn_calendar_countries = self
            .fn_calendar_countries
            .ok_or(Mt5Error::UnsupportedFeature("CalendarCountries"))?;

        let capacity = 16 * 1024 * 1024;
        let mut buf = vec![0u8; capacity];

        let count =
            unsafe { fn_calendar_countries(buf.as_mut_ptr() as *mut c_char, buf.len() as c_int) };
        if count < 0 {
            return Err(Mt5Error::CalendarCountriesFailed(count));
        }

        let mut countries = Vec::new();
        let mut off = 0;
        while off + 4 <= buf.len() {
            let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
            if len == 0 || off + 4 + len > buf.len() {
                break;
            }
            countries.push(String::from_utf8_lossy(&buf[off + 4..off + 4 + len]).to_string());
            off += 4 + len;
        }
        Ok(countries)
    }
}

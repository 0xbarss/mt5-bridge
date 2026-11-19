use crate::error::{Mt5Error, Result};
use crate::ffi::*;
use crate::types::*;
use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::time::{Duration, Instant};

use super::client::Mt5Client;

/// In-memory cache time-to-live for static symbol information.
pub const DEFAULT_SYMBOL_CACHE_TTL: Duration = Duration::from_secs(60);

impl Mt5Client {
    /// Set a custom cache TTL for symbol specifications.
    pub fn set_symbol_cache_ttl(&mut self, ttl: Duration) {
        self.symbol_cache_ttl = ttl;
    }

    /// Query current account balance, equity, and margin information.
    pub fn account_info(&self) -> Result<AccountInfo> {
        let (mut balance, mut equity, mut margin, mut free_margin, mut leverage) =
            (0.0, 0.0, 0.0, 0.0, 0.0);
        let mut currency = [0i8; 32];
        let ret = unsafe {
            (self.fn_acct)(
                &mut balance,
                &mut equity,
                &mut margin,
                &mut free_margin,
                &mut leverage,
                currency.as_mut_ptr(),
                currency.len() as c_int,
            )
        };

        if ret != 1 {
            return Err(Mt5Error::AccountInfoFailed(ret));
        }

        // The currency is a NUL-terminated, fixed-size field.
        let currency = unsafe { CStr::from_ptr(currency.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();

        Ok(AccountInfo {
            balance,
            equity,
            margin,
            free_margin,
            leverage,
            currency,
        })
    }

    /// Query symbol specifications (point size, tick value, lot sizes, spread, digits).
    ///
    /// Results are cached locally in-memory for `symbol_cache_ttl` duration.
    pub fn symbol_info(&self, symbol: &str) -> Result<SymbolInfo> {
        let upper = symbol.to_uppercase();

        if let Some((info, timestamp)) = self
            .symbol_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&upper)
            .cloned()
        {
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
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(upper, (info.clone(), Instant::now()));

        Ok(info)
    }

    /// Clear all cached symbol specifications.
    pub fn clear_symbol_cache(&self) {
        self.symbol_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

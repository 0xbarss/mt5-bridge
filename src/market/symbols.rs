//! Symbol universe discovery and registry.
//!
//! Provides [`SymbolRegistry`] — an in-memory, refreshable cache of MT5 symbols
//! fetched via [`Mt5Client::symbols`]/[`Mt5Client::symbols_filtered`] — plus
//! helper methods for filtering and selecting symbols in Market Watch.

use crate::error::{Mt5Error, Result};
use crate::types::SymbolInfo;
use std::sync::RwLock;

/// In-memory registry of MT5 symbols discovered through the bridge client.
///
/// The registry holds a cached snapshot of [`SymbolInfo`] entries and can be
/// refreshed from the MT5 terminal at any time. A refresh callback is supplied
/// at construction time so the registry stays decoupled from the client.
///
/// # Refresh-on-reconnect
///
/// Call [`refresh`](Self::refresh) after a reconnect to repopulate the cache
/// from the (possibly changed) Market Watch symbol list.
pub struct SymbolRegistry {
    symbols: RwLock<Vec<SymbolInfo>>,
    refresh_fn: Box<dyn Fn() -> Result<Vec<SymbolInfo>> + Send + Sync>,
}

impl SymbolRegistry {
    /// Create a new registry with `refresh_fn` used to fetch fresh symbols from MT5.
    pub fn new<F>(refresh_fn: F) -> Self
    where
        F: Fn() -> Result<Vec<SymbolInfo>> + Send + Sync + 'static,
    {
        Self {
            symbols: RwLock::new(Vec::new()),
            refresh_fn: Box::new(refresh_fn),
        }
    }

    /// Refresh the symbol cache from MT5, returning the number of symbols cached.
    pub fn refresh(&self) -> Result<usize> {
        let symbols = (self.refresh_fn)()?;
        let count = symbols.len();
        *self.symbols.write().unwrap() = symbols;
        Ok(count)
    }

    /// Return all cached symbols.
    pub fn symbols(&self) -> Vec<SymbolInfo> {
        self.symbols.read().unwrap().clone()
    }

    /// Return cached symbols matching predicate `f`.
    pub fn symbols_filtered(&self, f: impl Fn(&SymbolInfo) -> bool) -> Vec<SymbolInfo> {
        self.symbols
            .read()
            .unwrap()
            .iter()
            .filter(|s| f(s))
            .cloned()
            .collect()
    }

    /// Find a cached symbol by name.
    pub fn find(&self, symbol: &str) -> Option<SymbolInfo> {
        self.symbols
            .read()
            .unwrap()
            .iter()
            .find(|s| s.symbol == symbol)
            .cloned()
    }

    /// Number of cached symbols.
    pub fn len(&self) -> usize {
        self.symbols.read().unwrap().len()
    }

    /// True if no symbols are cached.
    pub fn is_empty(&self) -> bool {
        self.symbols.read().unwrap().is_empty()
    }
}

impl Default for SymbolRegistry {
    fn default() -> Self {
        Self::new(|| Err(Mt5Error::UnsupportedFeature("symbols")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_info(symbol: &str) -> SymbolInfo {
        SymbolInfo {
            symbol: symbol.into(),
            ..Default::default()
        }
    }

    #[test]
    fn registry_refresh_and_filter() {
        let registry = SymbolRegistry::new(|| Ok(vec![make_info("EURUSD"), make_info("GBPUSD")]));

        assert_eq!(registry.refresh().unwrap(), 2);
        assert_eq!(registry.len(), 2);
        assert!(registry.find("EURUSD").is_some());
        assert!(registry.find("XYZ").is_none());

        let filtered: Vec<_> = registry.symbols_filtered(|s| s.symbol.starts_with("EUR"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].symbol, "EURUSD");
    }

    #[test]
    fn registry_default_errors_on_refresh() {
        let registry = SymbolRegistry::default();
        assert!(registry.refresh().is_err());
    }

    #[test]
    fn registry_empty_before_refresh() {
        let registry = SymbolRegistry::new(|| Ok(vec![]));
        assert!(registry.is_empty());
        assert_eq!(registry.refresh().unwrap(), 0);
        assert!(registry.is_empty());
    }
}

use crate::error::{Mt5Error, Result};
use crate::types::*;
use std::ffi::CString;
use std::sync::Arc;

use super::client::Mt5Client;

impl Mt5Client {
    /// Subscribe to real-time pushed market data ticks for a symbol (protocol v5+ push model).
    ///
    /// By default uses [`StreamMode::Latest`] which drops stale quotes on consumer lag to prioritize lowest latency.
    pub fn subscribe_ticks(&self, symbol: &str) -> Result<crate::stream::TickSubscription> {
        self.subscribe_ticks_with_mode(symbol, StreamMode::Latest)
    }

    /// Subscribe to real-time pushed market data ticks for a symbol with an explicit [`StreamMode`] (protocol v5+).
    pub fn subscribe_ticks_with_mode(
        &self,
        symbol: &str,
        mode: StreamMode,
    ) -> Result<crate::stream::TickSubscription> {
        let fn_sub = self
            .fn_subscribe_ticks
            .ok_or(Mt5Error::UnsupportedFeature("SubscribeTicks"))?;
        let sym_c = CString::new(symbol.trim())?;
        let symbol = symbol.trim();

        // Subscribe the bus first so a tick pushed between bus registration and the
        // EA acknowledgment cannot be lost, then ask the EA to start pushing.
        let rx = self.event_bus.subscribe_ticks(symbol);
        let ret = unsafe { fn_sub(sym_c.as_ptr()) };
        if ret != 1 {
            self.event_bus.remove_tick_sender(symbol);
            return Err(Mt5Error::Other(format!(
                "SubscribeTicks failed for {symbol} (status={ret})"
            )));
        }
        self.register_tick_subscription(symbol);
        Ok(crate::stream::TickSubscription::new(symbol, mode, rx))
    }

    /// Unsubscribe from real-time pushed ticks for a symbol (protocol v5+).
    pub fn unsubscribe_ticks(&self, symbol: &str) -> Result<()> {
        let fn_unsub = self
            .fn_unsubscribe_ticks
            .ok_or(Mt5Error::UnsupportedFeature("UnsubscribeTicks"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_unsub(sym_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "UnsubscribeTicks failed for {symbol} (status={ret})"
            )));
        }
        self.unregister_tick_subscription(symbol.trim());
        self.event_bus.remove_tick_sender(symbol.trim());
        Ok(())
    }

    /// Subscribe to asynchronous broker trade transaction events (protocol v5+).
    ///
    /// Returns a [`TradeSubscription`](crate::stream::TradeSubscription) wrapping the broadcast
    /// receiver with explicit consumer-side lag detection (`recv_checked()` returns
    /// [`crate::stream::StreamError::Lagged`] when the broadcast buffer overflows).
    pub fn subscribe_trade(&self) -> Result<crate::stream::TradeSubscription> {
        let fn_sub = self
            .fn_subscribe_trade
            .ok_or(Mt5Error::UnsupportedFeature("SubscribeTrade"))?;
        let ret = unsafe { fn_sub() };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "SubscribeTrade failed (status={ret})"
            )));
        }
        self.register_trade_subscription();
        Ok(crate::stream::TradeSubscription::new(
            self.event_bus.subscribe_trade(),
        ))
    }

    /// Unsubscribe from asynchronous broker trade transaction events (protocol v5+).
    pub fn unsubscribe_trade(&self) -> Result<()> {
        let fn_unsub = self
            .fn_unsubscribe_trade
            .ok_or(Mt5Error::UnsupportedFeature("UnsubscribeTrade"))?;
        let ret = unsafe { fn_unsub() };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "UnsubscribeTrade failed (status={ret})"
            )));
        }
        self.unregister_trade_subscription();
        Ok(())
    }

    /// Subscribe to depth-of-market book events for a symbol (protocol v5+).
    pub fn subscribe_book(&self, symbol: &str) -> Result<crate::stream::BookSubscription> {
        let fn_sub = self
            .fn_subscribe_book
            .ok_or(Mt5Error::UnsupportedFeature("SubscribeBook"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_sub(sym_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "SubscribeBook failed for {symbol} (status={ret})"
            )));
        }
        self.register_book_subscription(symbol.trim());
        let rx = self.event_bus.subscribe_book(symbol.trim());
        Ok(crate::stream::BookSubscription::with_bus(
            symbol.trim(),
            rx,
            Arc::clone(&self.event_bus),
        ))
    }

    /// Unsubscribe from depth-of-market book events for a symbol (protocol v5+).
    pub fn unsubscribe_book(&self, symbol: &str) -> Result<()> {
        let fn_unsub = self
            .fn_unsubscribe_book
            .ok_or(Mt5Error::UnsupportedFeature("UnsubscribeBook"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_unsub(sym_c.as_ptr()) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "UnsubscribeBook failed for {symbol} (status={ret})"
            )));
        }
        self.unregister_book_subscription(symbol.trim());
        self.event_bus.prune_book(symbol.trim());
        Ok(())
    }

    /// Subscribe to real-time pushed OHLCV bars for a symbol and timeframe (protocol v7+).
    pub fn subscribe_bars(
        &self,
        symbol: &str,
        timeframe: Timeframe,
    ) -> Result<tokio::sync::broadcast::Receiver<Bar>> {
        let tf_raw = timeframe.to_mt5_const();
        let fn_sub = self
            .fn_subscribe_bars
            .ok_or(Mt5Error::UnsupportedFeature("SubscribeBars"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_sub(sym_c.as_ptr(), tf_raw) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "SubscribeBars failed for {symbol} tf={tf_raw} (status={ret})"
            )));
        }
        self.register_bars_subscription(symbol.trim(), timeframe);
        Ok(self.event_bus.subscribe_bars(symbol.trim(), tf_raw))
    }

    /// Unsubscribe from real-time pushed OHLCV bars for a symbol and timeframe (protocol v7+).
    pub fn unsubscribe_bars(&self, symbol: &str, timeframe: Timeframe) -> Result<()> {
        let tf_raw = timeframe.to_mt5_const();
        let fn_unsub = self
            .fn_unsubscribe_bars
            .ok_or(Mt5Error::UnsupportedFeature("UnsubscribeBars"))?;
        let sym_c = CString::new(symbol.trim())?;
        let ret = unsafe { fn_unsub(sym_c.as_ptr(), tf_raw) };
        if ret != 1 {
            return Err(Mt5Error::Other(format!(
                "UnsubscribeBars failed for {symbol} tf={tf_raw} (status={ret})"
            )));
        }
        self.unregister_bars_subscription(symbol.trim(), timeframe);
        Ok(())
    }

    /// Access the underlying [`EventBus`](crate::stream::EventBus) directly.
    pub fn event_bus(&self) -> Arc<crate::stream::EventBus> {
        Arc::clone(&self.event_bus)
    }

    /// Returns the symbols currently tracked in the active book subscription table.
    pub fn active_book_symbols(&self) -> Vec<String> {
        self.event_bus.active_book_symbols()
    }

    /// Total events dropped by the native C++ DLL buffer queue due to overflow.
    pub fn events_dropped_total(&self) -> u64 {
        if let Some(f) = self.fn_events_dropped_total {
            unsafe { f() }
        } else {
            0
        }
    }
}

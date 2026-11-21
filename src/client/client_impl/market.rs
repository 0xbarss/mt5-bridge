use crate::error::{Mt5Error, Result};
use crate::ffi::*;
use crate::types::*;
use chrono::{Datelike, TimeZone, Utc};
use std::ffi::CString;
use std::os::raw::c_int;
use std::thread;
use std::time::Duration;
use tracing::warn;

use super::client::Mt5Client;

fn range_overlaps_weekday(start: i64, end: i64) -> bool {
    let start = match Utc.timestamp_opt(start, 0).single() {
        Some(value) => value,
        None => return false,
    };
    let end = match Utc.timestamp_opt(end, 0).single() {
        Some(value) => value,
        None => return false,
    };
    let mut day = start.date_naive();
    let last_day = end.date_naive();
    while day <= last_day {
        if !matches!(day.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
            return true;
        }
        day = match day.succ_opt() {
            Some(next) => next,
            None => break,
        };
    }
    false
}

impl Mt5Client {
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
                "Requested range ({diff_bars} bars) exceeds maximum single fetch limit of {MAX_SINGLE_FETCH_BARS} bars. Please use copy_rates_chunked() for large historical datasets."
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
        self.copy_rates_chunked_cancellable(symbol, timeframe, start, end, chunk_bars, None)
    }

    /// Robust, chunked historical data downloader with cancellation token support.
    pub fn copy_rates_chunked_cancellable(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start: i64,
        end: i64,
        chunk_bars: usize,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<HistoryResult> {
        if start > end {
            return Err(Mt5Error::InvalidTimeRange { start, end });
        }

        let sym_c = CString::new(symbol)?;
        let tf_const = timeframe.to_mt5_const();
        let tf_secs = timeframe.seconds().max(1);

        const MT5_MAX_BARS_PER_FETCH: usize = 100_000;
        let chunk_size = chunk_bars.clamp(100, MT5_MAX_BARS_PER_FETCH);
        let chunk_duration = (chunk_size as i64).saturating_mul(tf_secs);

        let mut all_rates: Vec<Rate> = Vec::new();
        let mut missing_ranges: Vec<(i64, i64)> = Vec::new();
        let mut current_start = start;

        while current_start <= end {
            if let Some(cancel_token) = cancel {
                if cancel_token.load(std::sync::atomic::Ordering::Relaxed) {
                    all_rates.sort_by_key(|r| r.time);
                    all_rates.dedup_by_key(|r| r.time);
                    return Ok(HistoryResult {
                        rates: all_rates,
                        complete: false,
                        missing_ranges,
                        cancelled: true,
                    });
                }
            }

            let current_end = current_start.saturating_add(chunk_duration).min(end);
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
                        if range_overlaps_weekday(current_start, current_end) {
                            warn!(
                                symbol = symbol,
                                start = current_start,
                                end = current_end,
                                "CopyRates returned no bars during a weekday range; tracking as missing"
                            );
                            chunk_failed = true;
                        }
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
                thread::sleep(Duration::from_millis(100));
            }

            if chunk_failed {
                missing_ranges.push((current_start, current_end));
            }

            let filtered_chunk: Vec<Rate> = chunk_result
                .into_iter()
                .filter(|r| r.time >= current_start && r.time <= current_end)
                .collect();

            all_rates.extend(filtered_chunk);
            if current_end == i64::MAX {
                break;
            }
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
            cancelled: false,
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
}

#[cfg(test)]
mod tests {
    use super::range_overlaps_weekday;
    use chrono::{Datelike, TimeZone, Utc};

    #[test]
    fn empty_weekday_ranges_are_treated_as_missing() {
        let monday = Utc
            .with_ymd_and_hms(2026, 11, 2, 9, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("valid timestamp"));
        let monday_later = monday + chrono::Duration::hours(2);

        assert!(range_overlaps_weekday(
            monday.timestamp(),
            monday_later.timestamp()
        ));
        assert_eq!(monday.weekday(), chrono::Weekday::Mon);
    }

    #[test]
    fn weekend_only_ranges_are_allowed_to_be_empty() {
        let saturday = Utc
            .with_ymd_and_hms(2026, 11, 7, 9, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("valid timestamp"));
        let sunday = saturday + chrono::Duration::hours(36);

        assert!(!range_overlaps_weekday(
            saturday.timestamp(),
            sunday.timestamp()
        ));
    }
}

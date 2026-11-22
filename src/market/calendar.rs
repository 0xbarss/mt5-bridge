//! Economic calendar events and news filtering.
//!
//! MT5 exposes calendar data through two wire commands:
//!   - `CMD_CALENDAR_EVENTS` (25): events in a UTC time window
//!   - `CMD_CALENDAR_COUNTRIES` (26): countries that have calendar events
//!
//! The DLL and the MQL5 EA pack each event as a little-endian record:
//!   [u64 id][i64 time][u32 country_len][country][u32 currency_len][currency]
//!   [u8 importance][u32 title_len][title][f64 actual][u8 actual_present]
//!   [f64 forecast][u8 forecast_present][f64 previous][u8 previous_present]
//!
//! `actual`/`forecast`/`previous` are `Option<f64>` because MT5 only publishes
//! them once released; an event whose value is still pending decodes to `None`.
//! (On the MQL5 side a value of exactly `0.0` is reported as `None`, since
//! MQL5 carries no separate release flag.)

use serde::{Deserialize, Serialize};

/// An MT5 economic calendar event.
///
/// `time` is UTC seconds. `importance` is 1 (low), 2 (medium), or 3 (high).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: u64,
    pub time: i64,
    pub country: String,
    pub currency: String,
    pub importance: u8,
    pub title: String,
    pub actual: Option<f64>,
    pub forecast: Option<f64>,
    pub previous: Option<f64>,
}

impl CalendarEvent {
    /// Returns `true` once the event's `actual` value has been published.
    pub fn is_released(&self) -> bool {
        self.actual.is_some()
    }
}

/// Windowed news filter used to decide whether trading a symbol should be
/// blocked around a high-impact calendar event for a relevant currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFilter {
    /// Symbols whose trading is blocked around news. Empty = all symbols.
    pub symbols: Vec<String>,
    /// Currencies whose news events block trading. Empty = all currencies.
    pub currencies: Vec<String>,
    /// Minimum importance that triggers a block (1 = low, 2 = medium, 3 = high).
    pub min_importance: u8,
    /// Minutes before an event at which blocking starts.
    pub before_minutes: i64,
    /// Minutes after an event at which blocking stops.
    pub after_minutes: i64,
    /// Master enable/disable switch.
    pub enabled: bool,
}

impl Default for NewsFilter {
    fn default() -> Self {
        Self {
            symbols: Vec::new(),
            currencies: Vec::new(),
            min_importance: 3,
            before_minutes: 30,
            after_minutes: 15,
            enabled: true,
        }
    }
}

impl NewsFilter {
    /// Constructs a filter with sensible defaults (high-impact only, blocking
    /// 30 minutes before and 15 minutes after each event, enabled).
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if trading `symbol` at `when` (UTC seconds) is blocked by
    /// a news event in `events`.
    pub fn is_blocked(&self, symbol: &str, when: i64, events: &[CalendarEvent]) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.symbols.is_empty() && !self.symbols.iter().any(|s| s == symbol) {
            return false;
        }
        let before_secs = self.before_minutes * 60;
        let after_secs = self.after_minutes * 60;
        for ev in events {
            if ev.importance < self.min_importance {
                continue;
            }
            if !self.currencies.is_empty() && !self.currencies.iter().any(|c| c == &ev.currency) {
                continue;
            }
            let start = ev.time.saturating_sub(before_secs);
            let end = ev.time.saturating_add(after_secs);
            if when >= start && when <= end {
                return true;
            }
        }
        false
    }
}

/// Parse a payload buffer produced by the EA for `CMD_CALENDAR_EVENTS`.
///
/// Returns the total count as the `Ok` value is *not* the count here — the
/// caller already learns the total from the FFI status code; this only decodes
/// the packed records present in `buf`.
pub fn parse_calendar_events(buf: &[u8]) -> Result<Vec<CalendarEvent>, String> {
    let mut events = Vec::new();
    let mut off = 0;
    while off < buf.len() {
        if off + 8 > buf.len() {
            return Err("truncated calendar event id".to_string());
        }
        let id = u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        off += 8;
        // An id of 0 marks the end of the stream: the buffer is zero-initialised
        // and MT5 calendar ids are positive, so trailing zero padding is not a real event.
        if id == 0 {
            break;
        }

        if off + 8 > buf.len() {
            return Err("truncated calendar event time".to_string());
        }
        let time = i64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        off += 8;

        let (country, n) = read_len_prefixed(&buf[off..])?;
        off += n;
        let (currency, n) = read_len_prefixed(&buf[off..])?;
        off += n;

        if off + 1 > buf.len() {
            return Err("truncated calendar event importance".to_string());
        }
        let importance = buf[off];
        off += 1;

        let (title, n) = read_len_prefixed(&buf[off..])?;
        off += n;

        let actual = read_f64_option(&buf[off..])?;
        off += 9;
        let forecast = read_f64_option(&buf[off..])?;
        off += 9;
        let previous = read_f64_option(&buf[off..])?;
        off += 9;

        events.push(CalendarEvent {
            id,
            time,
            country,
            currency,
            importance,
            title,
            actual,
            forecast,
            previous,
        });
    }
    Ok(events)
}

fn read_len_prefixed(buf: &[u8]) -> Result<(String, usize), String> {
    if buf.len() < 4 {
        return Err("truncated length-prefixed string length".to_string());
    }
    let len = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    if buf.len() < 4 + len {
        return Err("truncated length-prefixed string body".to_string());
    }
    let s = String::from_utf8_lossy(&buf[4..4 + len]).to_string();
    Ok((s, 4 + len))
}

fn read_f64_option(buf: &[u8]) -> Result<Option<f64>, String> {
    if buf.len() < 9 {
        return Err("truncated calendar f64 option".to_string());
    }
    let value = f64::from_le_bytes(buf[0..8].try_into().unwrap());
    let present = buf[8];
    Ok(if present != 0 { Some(value) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_event(ev: &CalendarEvent) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&ev.id.to_le_bytes());
        buf.extend_from_slice(&ev.time.to_le_bytes());
        let push_str = |buf: &mut Vec<u8>, s: &str| {
            let b = s.as_bytes();
            buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
            buf.extend_from_slice(b);
        };
        let push_f64_opt = |buf: &mut Vec<u8>, v: Option<f64>| match v {
            Some(v) => {
                buf.extend_from_slice(&v.to_le_bytes());
                buf.push(1);
            }
            None => {
                buf.extend_from_slice(&0.0f64.to_le_bytes());
                buf.push(0);
            }
        };
        push_str(&mut buf, &ev.country);
        push_str(&mut buf, &ev.currency);
        buf.push(ev.importance);
        push_str(&mut buf, &ev.title);
        push_f64_opt(&mut buf, ev.actual);
        push_f64_opt(&mut buf, ev.forecast);
        push_f64_opt(&mut buf, ev.previous);
        buf
    }

    fn sample_events() -> Vec<CalendarEvent> {
        vec![
            CalendarEvent {
                id: 1,
                time: 1_700_000_000,
                country: "US".into(),
                currency: "USD".into(),
                importance: 3,
                title: "Non-Farm Payrolls".into(),
                actual: Some(200_000.0),
                forecast: Some(180_000.0),
                previous: Some(170_000.0),
            },
            CalendarEvent {
                id: 2,
                time: 1_700_003_600,
                country: "EU".into(),
                currency: "EUR".into(),
                importance: 2,
                title: "CPI".into(),
                actual: None,
                forecast: Some(2.5),
                previous: Some(2.4),
            },
            CalendarEvent {
                id: 3,
                time: 1_700_010_000,
                country: "JP".into(),
                currency: "JPY".into(),
                importance: 1,
                title: "Rate decision".into(),
                actual: None,
                forecast: None,
                previous: None,
            },
        ]
    }

    #[test]
    fn calendar_event_round_trip_through_packed_buffer() {
        let events = sample_events();
        let mut buf = Vec::new();
        for ev in &events {
            buf.extend_from_slice(&pack_event(ev));
        }
        let parsed = parse_calendar_events(&buf).expect("parse packed events");
        assert_eq!(parsed, events);
        assert!(parsed[0].is_released());
        assert!(!parsed[1].is_released());
    }

    #[test]
    fn parse_calendar_events_rejects_truncated_buffer() {
        let events = sample_events();
        let mut buf = Vec::new();
        for ev in &events {
            buf.extend_from_slice(&pack_event(ev));
        }
        buf.truncate(buf.len() - 3);
        assert!(parse_calendar_events(&buf).is_err());
    }

    #[test]
    fn news_filter_blocks_high_impact_event_window() {
        let filter = NewsFilter {
            enabled: true,
            min_importance: 3,
            before_minutes: 30,
            after_minutes: 15,
            ..Default::default()
        };
        let events = sample_events();
        let nfp = events[0].time; // high impact at 1700000000

        // well before the window
        assert!(!filter.is_blocked("EURUSD", nfp - 31 * 60, &events));
        // inside the before window
        assert!(filter.is_blocked("EURUSD", nfp - 10 * 60, &events));
        // at the event time
        assert!(filter.is_blocked("EURUSD", nfp, &events));
        // inside the after window
        assert!(filter.is_blocked("EURUSD", nfp + 10 * 60, &events));
        // just after the after window
        assert!(!filter.is_blocked("EURUSD", nfp + 16 * 60, &events));
    }

    #[test]
    fn news_filter_respects_importance_and_currency() {
        let filter = NewsFilter {
            enabled: true,
            min_importance: 3,
            currencies: vec!["USD".into()],
            ..Default::default()
        };
        let events = sample_events();
        // event 2 is EUR medium importance -> not blocked (wrong currency + below threshold)
        assert!(!filter.is_blocked("EURUSD", events[1].time, &events));
        // event 3 is JPY low importance -> not blocked
        assert!(!filter.is_blocked("USDJPY", events[2].time, &events));
        // event 1 is USD high importance -> blocked
        assert!(filter.is_blocked("EURUSD", events[0].time, &events));
    }

    #[test]
    fn news_filter_disabled_and_symbol_scoping() {
        let mut filter = NewsFilter::new();
        let events = sample_events();
        assert!(filter.is_blocked("XAUUSD", events[0].time, &events));

        filter.enabled = false;
        assert!(!filter.is_blocked("XAUUSD", events[0].time, &events));

        filter.enabled = true;
        filter.symbols = vec!["GBPUSD".into()];
        assert!(!filter.is_blocked("XAUUSD", events[0].time, &events));
        assert!(filter.is_blocked("GBPUSD", events[0].time, &events));
    }
}

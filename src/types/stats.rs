use crate::ffi::Mt5Stats;

/// High-level bridge statistics snapshot returned by [`Mt5Client::stats`](crate::Mt5Client::stats).
///
/// Mirrors the wire-packed [`Mt5Stats`](crate::ffi::Mt5Stats) struct returned by
/// `CMD_STATS` (27). All counters are monotonic since the bridge DLL was loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeStats {
    /// Seconds since the bridge DLL was loaded.
    pub uptime_secs: u64,
    /// Total number of requests sent to the MT5 EA.
    pub requests_total: u64,
    /// Tick events dropped due to a full event queue.
    pub tick_drops: u64,
    /// Trade events dropped due to a full event queue.
    pub trade_drops: u64,
    /// Book events dropped due to a full event queue.
    pub book_drops: u64,
    /// Bar events dropped due to a full event queue.
    pub bar_drops: u64,
    /// Current number of queued tick events awaiting dispatch.
    pub tick_queue_depth: u32,
    /// Current number of queued trade events awaiting dispatch.
    pub trade_queue_depth: u32,
    /// Last error status code reported by the bridge (0 when no error).
    pub last_error: i32,
    /// Wire protocol version negotiated with the EA.
    pub protocol: u32,
}

impl BridgeStats {
    /// Decodes a wire-packed [`Mt5Stats`](crate::ffi::Mt5Stats) into [`BridgeStats`].
    pub fn from_raw(raw: Mt5Stats) -> Self {
        Self {
            uptime_secs: raw.uptime_secs,
            requests_total: raw.requests_total,
            tick_drops: raw.tick_drops,
            trade_drops: raw.trade_drops,
            book_drops: raw.book_drops,
            bar_drops: raw.bar_drops,
            tick_queue_depth: raw.tick_queue_depth,
            trade_queue_depth: raw.trade_queue_depth,
            last_error: raw.last_error,
            protocol: raw.protocol,
        }
    }
}

impl Default for BridgeStats {
    fn default() -> Self {
        Self {
            uptime_secs: 0,
            requests_total: 0,
            tick_drops: 0,
            trade_drops: 0,
            book_drops: 0,
            bar_drops: 0,
            tick_queue_depth: 0,
            trade_queue_depth: 0,
            last_error: 0,
            protocol: crate::ffi::PROTOCOL_VERSION,
        }
    }
}

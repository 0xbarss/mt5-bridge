use crate::error::{Mt5Error, Result};
use crate::ffi::*;
use crate::types::*;
use std::ffi::CString;
use std::os::raw::c_int;
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
}

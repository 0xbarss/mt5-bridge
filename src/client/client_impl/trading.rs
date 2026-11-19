use crate::error::{mt5_retcode_description, Mt5Error, Result};
use crate::ffi::*;
use crate::types::*;
use std::ffi::CString;
use std::os::raw::{c_double, c_int};
use tracing::debug;

use super::client::Mt5Client;

impl Mt5Client {
    /// Send a trading order (Market Buy/Sell or Limit/Stop orders).
    pub fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        // `validate_for_submit` additionally requires a stop loss: an order
        // without one would be placed unprotected.
        req.validate_for_submit()?;

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

    /// Close an existing position by ticket number, specifying custom deviation and volume limits.
    pub fn order_close_with_deviation(
        &self,
        ticket: u64,
        magic: u64,
        deviation: u32,
        volume: f64,
    ) -> Result<TradeResult> {
        if ticket == 0 {
            return Err(Mt5Error::Other("Order ticket cannot be 0".to_string()));
        }

        let mut res = Mt5TradeResult::default();
        let ret = if let Some(fn_close_dev) = self.fn_close_deviation {
            unsafe { fn_close_dev(ticket, magic, deviation, volume, &mut res) }
        } else if let Some(fn_close_mag) = self.fn_close_magic {
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
            "Position closed successfully with deviation"
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

    /// Close an open market position by its position ticket number.
    ///
    /// This is an ergonomic counterpart to [`order_cancel`](Self::order_cancel) specifically intended
    /// for market positions. Under the hood, the EA detects that the ticket represents an open
    /// position and executes an opposing market deal (`TRADE_ACTION_DEAL`) to close it.
    pub fn close_position(&self, ticket: u64) -> Result<TradeResult> {
        self.order_close(ticket)
    }

    /// Close an open market position by its position ticket number, verifying strategy magic number ownership.
    pub fn close_position_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        self.order_close_with_magic(ticket, magic)
    }

    /// Modify the Stop Loss and/or Take Profit of an open position or pending order.
    pub fn order_modify(
        &self,
        ticket: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        self.order_modify_with_magic(ticket, 0, stop_loss, take_profit)
    }

    /// Modify the Stop Loss and/or Take Profit of an open position or pending order,
    /// verifying that it belongs to the given magic number.
    pub fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        if ticket == 0 {
            return Err(Mt5Error::Other("Order ticket cannot be 0".to_string()));
        }
        if let Level::Set(sl) = stop_loss {
            if sl < 0.0 || sl.is_nan() || sl.is_infinite() {
                return Err(Mt5Error::Other(format!(
                    "Invalid stop loss {sl}: stop loss cannot be negative or NaN"
                )));
            }
        }
        if let Level::Set(tp) = take_profit {
            if tp < 0.0 || tp.is_nan() || tp.is_infinite() {
                return Err(Mt5Error::Other(format!(
                    "Invalid take profit {tp}: take profit cannot be negative or NaN"
                )));
            }
        }

        let (sl_mode, sl_val) = stop_loss.to_wire();
        let (tp_mode, tp_val) = take_profit.to_wire();

        let mut res = Mt5TradeResult::default();
        let ret = if let Some(fn_mod_mag) = self.fn_modify_magic {
            unsafe {
                fn_mod_mag(
                    ticket,
                    magic,
                    sl_mode,
                    sl_val as c_double,
                    tp_mode,
                    tp_val as c_double,
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
                    sl_mode,
                    sl_val as c_double,
                    tp_mode,
                    tp_val as c_double,
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
            sl = ?stop_loss,
            tp = ?take_profit,
            retcode = trade_result.retcode,
            "Order modified successfully"
        );
        Ok(trade_result)
    }
}

use crate::error::{Mt5Error, Result};
use crate::types::order_enums::OrderType;
use crate::types::symbol::SymbolInfo;
use crate::types::wire_id::{
    truncate_utf8, wire_id, MAX_CLIENT_ORDER_ID_BYTES, MT5_COMMENT_MAX_BYTES, WIRE_COMMENT_PREFIX,
};
use crate::types::{Magic, Price, Symbol, Volume};
use serde::{Deserialize, Serialize};

/// Parameters for placing an order via `order_send`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub order_type: OrderType,
    pub volume: f64,
    pub price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub comment: String,
    pub deviation: Option<u32>,
    pub expiration: Option<i64>,
    pub magic: Option<u64>,
    pub client_order_id: Option<String>,
}

impl OrderRequest {
    /// Create a new market Buy order request.
    pub fn buy(symbol: impl Into<String>, volume: f64) -> Self {
        Self {
            symbol: symbol.into(),
            order_type: OrderType::Buy,
            volume,
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Create a new market Sell order request.
    pub fn sell(symbol: impl Into<String>, volume: f64) -> Self {
        Self {
            symbol: symbol.into(),
            order_type: OrderType::Sell,
            volume,
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Create a new limit or pending order request.
    pub fn pending(
        symbol: impl Into<String>,
        order_type: OrderType,
        volume: f64,
        price: f64,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            order_type,
            volume,
            price,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    pub fn stop_loss(mut self, sl: f64) -> Self {
        self.stop_loss = sl;
        self
    }

    pub fn take_profit(mut self, tp: f64) -> Self {
        self.take_profit = tp;
        self
    }

    pub fn price(mut self, price: f64) -> Self {
        self.price = price;
        self
    }

    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = comment.into();
        self
    }

    pub fn deviation(mut self, deviation: u32) -> Self {
        self.deviation = Some(deviation);
        self
    }

    pub fn expiration(mut self, expiration: i64) -> Self {
        self.expiration = Some(expiration);
        self
    }

    pub fn magic(mut self, magic: u64) -> Self {
        self.magic = Some(magic);
        self
    }

    pub fn client_order_id(mut self, id: impl Into<String>) -> Self {
        self.client_order_id = Some(id.into());
        self
    }

    // ---- Typed constructors (alongside the primitive API) ----

    /// Create a new market Buy order request with typed units.
    pub fn buy_typed(symbol: impl Into<Symbol>, volume: Volume) -> Self {
        Self {
            symbol: symbol.into().into_inner(),
            order_type: OrderType::Buy,
            volume: volume.into_inner(),
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Create a new market Sell order request with typed units.
    pub fn sell_typed(symbol: impl Into<Symbol>, volume: Volume) -> Self {
        Self {
            symbol: symbol.into().into_inner(),
            order_type: OrderType::Sell,
            volume: volume.into_inner(),
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Create a new limit or pending order request with typed units.
    pub fn pending_typed(
        symbol: impl Into<Symbol>,
        order_type: OrderType,
        volume: Volume,
        price: Price,
    ) -> Self {
        Self {
            symbol: symbol.into().into_inner(),
            order_type,
            volume: volume.into_inner(),
            price: price.into_inner(),
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    // ---- Typed field mutators (alongside the primitive API) ----

    pub fn symbol_typed(mut self, symbol: Symbol) -> Self {
        self.symbol = symbol.into_inner();
        self
    }

    pub fn volume_typed(mut self, volume: Volume) -> Self {
        self.volume = volume.into_inner();
        self
    }

    pub fn price_typed(mut self, price: Price) -> Self {
        self.price = price.into_inner();
        self
    }

    pub fn stop_loss_typed(mut self, stop_loss: Price) -> Self {
        self.stop_loss = stop_loss.into_inner();
        self
    }

    pub fn take_profit_typed(mut self, take_profit: Price) -> Self {
        self.take_profit = take_profit.into_inner();
        self
    }

    pub fn magic_typed(mut self, magic: Magic) -> Self {
        self.magic = Some(magic.into_inner());
        self
    }

    // ---- Typed field accessors (alongside the public primitive fields) ----

    pub fn symbol_typed_get(&self) -> Symbol {
        Symbol::from(self.symbol.clone())
    }

    pub fn volume_typed_get(&self) -> Volume {
        Volume(self.volume)
    }

    pub fn price_typed_get(&self) -> Price {
        Price(self.price)
    }

    pub fn stop_loss_typed_get(&self) -> Price {
        Price(self.stop_loss)
    }

    pub fn take_profit_typed_get(&self) -> Price {
        Price(self.take_profit)
    }

    pub fn magic_typed_get(&self) -> Option<Magic> {
        self.magic.map(Magic)
    }

    // ---- Builder API ----

    /// Create a builder for a new market Buy order request.
    ///
    /// Chain configuration methods and call [`build`](OrderRequestBuilder::build)
    /// to produce a validated [`OrderRequest`].
    pub fn market_buy(symbol: impl Into<String>, volume: Volume) -> OrderRequestBuilder {
        OrderRequestBuilder {
            symbol: symbol.into(),
            order_type: OrderType::Buy,
            volume: volume.into_inner(),
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Create a builder for a new market Sell order request.
    ///
    /// Chain configuration methods and call [`build`](OrderRequestBuilder::build)
    /// to produce a validated [`OrderRequest`].
    pub fn market_sell(symbol: impl Into<String>, volume: Volume) -> OrderRequestBuilder {
        OrderRequestBuilder {
            symbol: symbol.into(),
            order_type: OrderType::Sell,
            volume: volume.into_inner(),
            price: 0.0,
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }
}

/// Builder for constructing an [`OrderRequest`] with validation on `build()`.
///
/// Start with [`OrderRequest::market_buy`] or [`OrderRequest::market_sell`],
/// chain configuration methods, then call [`build`](Self::build) which runs
/// [`OrderRequest::validate`] before returning the request.
///
/// ```rust
/// use mt5_bridge::{OrderRequest, Price, Volume};
///
/// let req = OrderRequest::market_buy("EURUSD", Volume::lots(0.10))
///     .stop_loss(Price(1.0800))
///     .take_profit(Price(1.0900))
///     .deviation(20)
///     .comment("breakout")
///     .client_order_id("eurusd-breakout-001")
///     .build()
///     .expect("valid order request");
/// assert_eq!(req.symbol, "EURUSD");
/// assert_eq!(req.volume, 0.10);
/// assert_eq!(req.stop_loss, 1.0800);
/// assert_eq!(req.take_profit, 1.0900);
/// assert_eq!(req.deviation, Some(20));
/// assert_eq!(req.comment, "breakout");
/// assert_eq!(req.client_order_id.as_deref(), Some("eurusd-breakout-001"));
/// ```
#[derive(Debug, Clone)]
pub struct OrderRequestBuilder {
    symbol: String,
    order_type: OrderType,
    volume: f64,
    price: f64,
    stop_loss: f64,
    take_profit: f64,
    comment: String,
    deviation: Option<u32>,
    expiration: Option<i64>,
    magic: Option<u64>,
    client_order_id: Option<String>,
}

impl OrderRequestBuilder {
    /// Create a builder for a pending (limit/stop) order request.
    pub fn pending(
        symbol: impl Into<String>,
        order_type: OrderType,
        volume: Volume,
        price: Price,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            order_type,
            volume: volume.into_inner(),
            price: price.into_inner(),
            stop_loss: 0.0,
            take_profit: 0.0,
            comment: String::new(),
            deviation: None,
            expiration: None,
            magic: None,
            client_order_id: None,
        }
    }

    /// Set the stop loss price.
    pub fn stop_loss(mut self, sl: Price) -> Self {
        self.stop_loss = sl.into_inner();
        self
    }

    /// Set the take profit price.
    pub fn take_profit(mut self, tp: Price) -> Self {
        self.take_profit = tp.into_inner();
        self
    }

    /// Set the order price (for pending orders).
    pub fn price(mut self, price: Price) -> Self {
        self.price = price.into_inner();
        self
    }

    /// Set the maximum deviation in points.
    pub fn deviation(mut self, deviation: u32) -> Self {
        self.deviation = Some(deviation);
        self
    }

    /// Set the order expiration (epoch milliseconds).
    pub fn expiration(mut self, expiration: i64) -> Self {
        self.expiration = Some(expiration);
        self
    }

    /// Set the EA magic number.
    pub fn magic(mut self, magic: Magic) -> Self {
        self.magic = Some(magic.into_inner());
        self
    }

    /// Set the order comment.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = comment.into();
        self
    }

    /// Set the client order ID.
    pub fn client_order_id(mut self, id: impl Into<String>) -> Self {
        self.client_order_id = Some(id.into());
        self
    }

    /// Build the [`OrderRequest`], running validation first.
    ///
    /// Returns an error if the request fails [`OrderRequest::validate`]
    /// (e.g. empty symbol, invalid volume, non-finite prices).
    pub fn build(self) -> Result<OrderRequest, Mt5Error> {
        let req = OrderRequest {
            symbol: self.symbol,
            order_type: self.order_type,
            volume: self.volume,
            price: self.price,
            stop_loss: self.stop_loss,
            take_profit: self.take_profit,
            comment: self.comment,
            deviation: self.deviation,
            expiration: self.expiration,
            magic: self.magic,
            client_order_id: self.client_order_id,
        };
        req.validate()?;
        Ok(req)
    }
}
impl OrderRequest {
    /// Formats the effective MT5 wire comment, embedding the order's identity if present.
    ///
    /// With a `client_order_id` the result is `cid:<wire_id>` or `cid:<wire_id>:<comment>`,
    /// where `<wire_id>` is the fixed-width hash from [`wire_id`] — **not** a prefix of the
    /// raw ID — so two different client order IDs can never be truncated into the same wire
    /// identity. The whole string is capped at [`MT5_COMMENT_MAX_BYTES`] bytes; only the
    /// free-text `comment` part is ever shortened, and always on a UTF-8 character boundary,
    /// so this function cannot panic on non-ASCII input.
    pub fn effective_comment(&self) -> String {
        let full = match &self.client_order_id {
            Some(cid) if !cid.is_empty() => {
                let token = wire_id(cid);
                if self.comment.is_empty() {
                    format!("{WIRE_COMMENT_PREFIX}{token}")
                } else {
                    format!("{WIRE_COMMENT_PREFIX}{token}:{}", self.comment)
                }
            }
            _ => self.comment.clone(),
        };
        truncate_utf8(&full, MT5_COMMENT_MAX_BYTES).to_string()
    }

    /// Validates the order request parameters before submitting to the MT5 bridge.
    pub fn validate(&self) -> Result<(), Mt5Error> {
        if let Some(cid) = &self.client_order_id {
            if cid.trim().is_empty() {
                return Err(Mt5Error::Other(
                    "client_order_id must not be empty or whitespace-only".to_string(),
                ));
            }
            if cid.len() > MAX_CLIENT_ORDER_ID_BYTES {
                return Err(Mt5Error::Other(format!(
                    "client_order_id is {} bytes; maximum is {MAX_CLIENT_ORDER_ID_BYTES}",
                    cid.len()
                )));
            }
            if cid.chars().any(char::is_control) {
                return Err(Mt5Error::Other(
                    "client_order_id must not contain control characters".to_string(),
                ));
            }
        }
        if self.comment.chars().any(char::is_control) {
            return Err(Mt5Error::Other(
                "order comment must not contain control characters".to_string(),
            ));
        }
        if self.client_order_id.is_none() && self.comment.starts_with(WIRE_COMMENT_PREFIX) {
            return Err(Mt5Error::Other(format!(
                "comment prefix '{WIRE_COMMENT_PREFIX}' is reserved for client order ID tracking"
            )));
        }
        if self.symbol.trim().is_empty() {
            return Err(Mt5Error::Other(
                "OrderRequest symbol cannot be empty".to_string(),
            ));
        }
        if self.volume <= 0.0 || self.volume.is_nan() || self.volume.is_infinite() {
            return Err(Mt5Error::Other(format!(
                "Invalid order volume {}: volume must be a positive finite number",
                self.volume
            )));
        }
        if self.price < 0.0 || self.price.is_nan() || self.price.is_infinite() {
            return Err(Mt5Error::Other(format!(
                "Invalid order price {}: price cannot be negative or NaN",
                self.price
            )));
        }
        if self.stop_loss < 0.0 || self.stop_loss.is_nan() || self.stop_loss.is_infinite() {
            return Err(Mt5Error::Other(format!(
                "Invalid stop loss {}: stop loss cannot be negative or NaN",
                self.stop_loss
            )));
        }
        if self.take_profit < 0.0 || self.take_profit.is_nan() || self.take_profit.is_infinite() {
            return Err(Mt5Error::Other(format!(
                "Invalid take profit {}: take profit cannot be negative or NaN",
                self.take_profit
            )));
        }
        Ok(())
    }

    /// Additional checks that apply to an order **on its way to the venue**.
    ///
    /// A stop loss is mandatory: `0.0` is the "no stop" sentinel the terminal
    /// understands, so an order with it would be placed unprotected. The
    /// Expert Advisor rejects such orders too, but the check belongs here as
    /// well — one layer before the wire.
    pub fn validate_for_submit(&self) -> Result<(), Mt5Error> {
        self.validate()?;
        if self.stop_loss <= 0.0 {
            return Err(Mt5Error::Other(format!(
                "Invalid stop loss {}: a stop loss is required for every order",
                self.stop_loss
            )));
        }
        Ok(())
    }

    /// Validates the order request against live symbol specifications:
    /// minimum/maximum lot sizes, lot step alignment, and tick size alignment.
    pub fn validate_with_symbol(&self, info: &SymbolInfo) -> Result<(), Mt5Error> {
        self.validate()?;
        if !info.is_valid_lot(self.volume) {
            return Err(Mt5Error::Other(format!(
                "Order volume {} does not satisfy symbol {} lot constraints (min={}, max={}, step={})",
                self.volume, self.symbol, info.min_lot, info.max_lot, info.lot_step
            )));
        }
        if info.tick_size > 0.0 {
            if self.price > 0.0 {
                let steps = self.price / info.tick_size;
                if (steps - steps.round()).abs() > 1e-4 {
                    return Err(Mt5Error::Other(format!(
                        "Order price {} is not aligned to tick size {}",
                        self.price, info.tick_size
                    )));
                }
            }
            if self.stop_loss > 0.0 {
                let steps = self.stop_loss / info.tick_size;
                if (steps - steps.round()).abs() > 1e-4 {
                    return Err(Mt5Error::Other(format!(
                        "Stop loss {} is not aligned to tick size {}",
                        self.stop_loss, info.tick_size
                    )));
                }
            }
            if self.take_profit > 0.0 {
                let steps = self.take_profit / info.tick_size;
                if (steps - steps.round()).abs() > 1e-4 {
                    return Err(Mt5Error::Other(format!(
                        "Take profit {} is not aligned to tick size {}",
                        self.take_profit, info.tick_size
                    )));
                }
            }
        }
        Ok(())
    }
}

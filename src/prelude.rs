//! Prelude for mt5-bridge.
//!
//! Re-exports the most commonly used types so consumers can write
//! `use mt5_bridge::prelude::*;` instead of importing each item individually.
//!
//! ```
//! use mt5_bridge::prelude::*;
//!
//! let req = OrderRequest::market_buy("EURUSD", Volume::lots(0.10))
//!     .stop_loss(Price(1.0800))
//!     .build()
//!     .expect("valid order");
//! ```

pub use crate::constants::EA_DEFAULT_MAGIC_TYPED;
pub use crate::error::{Mt5Error, Result};
pub use crate::types::{
    Magic, OrderRequest, OrderRequestBuilder, OrderType, Points, Price, Symbol, Ticket, Volume,
};

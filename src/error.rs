use thiserror::Error;

/// Result alias for MT5 bridge operations.
pub type Result<T, E = Mt5Error> = std::result::Result<T, E>;

/// Errors returned by the MetaTrader 5 bridge client.
#[derive(Debug, Error)]
pub enum Mt5Error {
    #[error("Failed to load MT5 bridge DLL '{path}': {source}")]
    DllLoadError {
        path: String,
        #[source]
        source: libloading::Error,
    },

    #[error("Required DLL symbol '{symbol}' was not found: {source}")]
    SymbolNotFound {
        symbol: &'static str,
        #[source]
        source: libloading::Error,
    },

    #[error("MT5 bridge initialization failed with status code {0}. Please ensure MT5 terminal is running, the EA is attached to an active chart, 'Allow DLL imports' is enabled, and credentials are correct.")]
    InitFailed(i32),

    #[error("CopyRates failed for symbol '{symbol}' (status {status})")]
    CopyRatesFailed { symbol: String, status: i32 },

    #[error("AccountInfo query failed (status {0})")]
    AccountInfoFailed(i32),

    #[error("SymbolInfoFull query failed for symbol '{0}'")]
    SymbolInfoFailed(String),

    #[error("SymbolInfoTick query failed for symbol '{0}'")]
    SymbolTickFailed(String),

    #[error("OrderSend failed for symbol '{symbol}' (retcode {retcode}: {description})")]
    OrderSendFailed {
        symbol: String,
        retcode: u32,
        description: &'static str,
    },

    #[error("OrderClose failed for ticket {ticket} (retcode {retcode}: {description})")]
    OrderCloseFailed {
        ticket: u64,
        retcode: u32,
        description: &'static str,
    },

    #[error("OrderModify failed for ticket {ticket} (retcode {retcode}: {description})")]
    OrderModifyFailed {
        ticket: u64,
        retcode: u32,
        description: &'static str,
    },

    #[error("Order execution status is UNKNOWN for symbol '{symbol}' (client_order_id: {client_order_id:?}): {description}. A response was not received from MT5; reconciliation against broker state is required before retrying.")]
    UnknownExecutionState {
        symbol: String,
        client_order_id: Option<String>,
        description: String,
    },

    #[error("Failed to transmit order request to MT5 bridge before execution: {0}")]
    TransmissionFailed(String),

    #[error("Positions query failed (status {0})")]
    PositionsFailed(i32),

    #[error("Working orders query failed (status {0})")]
    OrdersFailed(i32),

    #[error("Position or order ticket {ticket} has magic {actual_magic}, which does not match expected strategy magic {expected_magic}")]
    OwnershipMismatch {
        ticket: u64,
        expected_magic: u64,
        actual_magic: u64,
    },

    #[error("Reconciliation error: {0}")]
    ReconciliationError(String),

    #[error("Optional export '{0}' is not supported by the loaded DLL")]
    UnsupportedFeature(&'static str),

    #[error("String contains interior null byte: {0}")]
    InteriorNullByte(#[from] std::ffi::NulError),

    #[error("Invalid time range: start timestamp ({start}) is greater than end timestamp ({end})")]
    InvalidTimeRange { start: i64, end: i64 },

    #[error("Thread task join error: {0}")]
    JoinError(String),

    #[error("Stream channel disconnected")]
    ChannelDisconnected,

    #[error("Internal bridge error: {0}")]
    Other(String),
}

/// Helper function to translate standard MT5 retcodes to human-readable strings.
pub fn mt5_retcode_description(retcode: u32) -> &'static str {
    match retcode {
        10004 => "TRADE_RETCODE_REQUOTE: Requote",
        10006 => "TRADE_RETCODE_REJECT: Request rejected",
        10007 => "TRADE_RETCODE_CANCEL: Request canceled by trader",
        10008 => "TRADE_RETCODE_PLACED: Order placed",
        10009 => "TRADE_RETCODE_DONE: Request completed",
        10010 => "TRADE_RETCODE_DONE_PARTIAL: Only part of the request was completed",
        10011 => "TRADE_RETCODE_ERROR: Request processing error",
        10012 => "TRADE_RETCODE_TIMEOUT: Request canceled by timeout",
        10013 => "TRADE_RETCODE_INVALID: Invalid request",
        10014 => "TRADE_RETCODE_INVALID_VOLUME: Invalid volume in the request",
        10015 => "TRADE_RETCODE_INVALID_PRICE: Invalid price in the request",
        10016 => "TRADE_RETCODE_INVALID_STOPS: Invalid stops (SL/TP) in the request",
        10017 => "TRADE_RETCODE_TRADE_DISABLED: Trade is disabled",
        10018 => "TRADE_RETCODE_MARKET_CLOSED: Market is closed",
        10019 => "TRADE_RETCODE_NO_MONEY: There is not enough money to complete the request",
        10020 => "TRADE_RETCODE_PRICE_CHANGED: Prices changed",
        10021 => "TRADE_RETCODE_PRICE_OFF: There are no quotes to process the request",
        10022 => "TRADE_RETCODE_INVALID_EXPIRATION: Invalid order expiration date in the request",
        10023 => "TRADE_RETCODE_ORDER_CHANGED: Order state changed",
        10024 => "TRADE_RETCODE_TOO_MANY_REQUESTS: Too frequent requests",
        10025 => "TRADE_RETCODE_NO_CHANGES: No changes in request",
        10026 => "TRADE_RETCODE_SERVER_DISABLES_AT: Autotrading disabled by server",
        10027 => "TRADE_RETCODE_CLIENT_DISABLES_AT: Autotrading disabled by client terminal",
        10028 => "TRADE_RETCODE_LOCKED: Request locked for processing",
        10029 => "TRADE_RETCODE_FROZEN: Order or position frozen",
        10030 => "TRADE_RETCODE_INVALID_FILL: Invalid order execution type",
        10031 => "TRADE_RETCODE_CONNECTION: No connection with the trade server",
        10032 => "TRADE_RETCODE_ONLY_REAL: Operation is allowed only for live accounts",
        10033 => "TRADE_RETCODE_LIMIT_ORDERS: The number of pending orders has reached the limit",
        10034 => {
            "TRADE_RETCODE_LIMIT_VOLUME: The volume of orders and positions has reached the limit"
        }
        10035 => "TRADE_RETCODE_INVALID_ORDER: Incorrect or prohibited order type",
        10036 => "TRADE_RETCODE_POSITION_CLOSED: Position with specified ticket is already closed",
        10038 => {
            "TRADE_RETCODE_INVALID_CLOSE_VOLUME: Close volume exceeds the current position volume"
        }
        10039 => "TRADE_RETCODE_CLOSE_ORDER_EXIST: A close order already exists for this position",
        10040 => {
            "TRADE_RETCODE_LIMIT_POSITIONS: The number of open positions has reached the limit"
        }
        _ => "Unknown MT5 retcode",
    }
}

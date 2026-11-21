# MetaTrader 5 Binary IPC Protocol Specification

**Current Protocol Version:** `7` (`MT5_BRIDGE_PROTOCOL_VERSION`)

This document defines the authoritative binary wire protocol, packet framing, command table, and packed struct memory layouts for `mt5-bridge`.

---

## 1. Transport & Architecture Overview

Communication between the client (Rust / C++) and the MetaTrader 5 Expert Advisor (`mt5_bridge.mq5`) operates over a local **Windows Named Pipe** (`\\.\pipe\<name>`, default `\\.\pipe\mt5bridge`).

- **Framing:** Little-endian packed binary frames. Every packet begins with a fixed 16-byte `PacketHdr`.
- **Request Serialization:** The C++ DLL serializes outbound requests via `std::mutex` and condition variables, ensuring one request/response exchange is in flight at a time per pipe handle.
- **Asynchronous Event Pump:** High-frequency pushed events (ticks, book updates, trade transactions, closed bars) are dispatched by the EA into the pipe and received by the DLL background reader into an internal circular ring buffer (default 8,192 slots) before being drained by client consumers.
- **Single Client per Process:** The Rust client enforces a single active connection per process. Multi-account or multi-terminal deployments require one separate process per pipe.

---

## 2. Packet Header (`PacketHdr`)

Every binary packet across the pipe starts with a 16-byte header (`#pragma pack(push, 1)`):

```text
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                            length                             |  (4 bytes, uint32)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|     kind      |     _pad      |              id               |  (1+1+2 bytes)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                            status                             |  (4 bytes, int32)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                          request_id                           |  (4 bytes, uint32)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                           payload...                          |
```

### Fields

| Field | Type | Size | Description |
| :--- | :--- | :---: | :--- |
| `length` | `uint32_t` | 4 B | Total packet length including the 16-byte header. |
| `kind` | `uint8_t` | 1 B | Packet class: `PKT_REQUEST = 0`, `PKT_RESPONSE = 1`, `PKT_EVENT = 2`. |
| `_pad` | `uint8_t` | 1 B | Explicit byte alignment padding (must be 0). |
| `id` | `uint16_t` | 2 B | Command ID for requests/responses, or Event Type for events. |
| `status` | `int32_t` | 4 B | Execution status code (`0` for success; `< 0` for error). |
| `request_id` | `uint32_t` | 4 B | Unique monotonic correlation identifier for request-response matching. |

### Status Codes

| Code | Constant | Meaning |
| :---: | :--- | :--- |
| `0` | `SUCCESS` | Request succeeded; valid payload follows. |
| `-1` | `MT5_ERR_SEND_FAILED` | Transmission over pipe failed or timed out. |
| `-2` | `MT5_ERR_UNKNOWN_EXECUTION` | Network/pipe dropped after send; order state is ambiguous. |
| `-3` | `MT5_ERR_PIPE_DISCONNECTED` | Named pipe disconnected or handle broken. |
| `-4` | `MT5_ERR_SERVER_AUTH_FAILED` | Account credentials or secret token rejected. |
| `-5` | `MT5_ERR_EA_REJECTED` | MQL5 preflight validation rejected request (e.g. invalid lot/stops). |
| `-6` | `MT5_ERR_ALREADY_INITIALIZED` | Handshake already performed on this connection. |

---

## 3. Command Table

| ID | Constant | Direction | Payload Description |
| :---: | :--- | :---: | :--- |
| `1` | `CMD_INIT` | Req → Resp | Handshake: login (`int64`), secret (`string`), server (`string`), protocol version (`int32 = 7`). |
| `2` | `CMD_SHUTDOWN` | Req → Resp | Clean disconnect and pipe teardown. |
| `3` | `CMD_RATES` | Req → Resp | Historical OHLCV query: symbol, timeframe, from UTC, to UTC, max bars. |
| `4` | `CMD_ACCOUNT` | Req → Resp | Query account balance, equity, margin, free margin. |
| `5` | `CMD_ORDER_SEND` | Req → Resp | Send market or pending trade order with idempotency cache. |
| `6` | `CMD_ORDER_CLOSE` | Req → Resp | Close position or delete pending order by ticket ID. |
| `7` | `CMD_ORDER_MODIFY` | Req → Resp | Modify SL and TP levels on an open ticket. |
| `8` | `CMD_SYM_TICK` | Req → Resp | Query current bid/ask quote tick for a symbol. |
| `9` | `CMD_SYM_INFO` | Req → Resp | Query contract specifications and trading limits. |
| `10` | `CMD_POSITIONS_GET` | Req → Resp | Query all active open positions matching optional magic filter. |
| `11` | `CMD_ORDERS_GET` | Req → Resp | Query all working pending orders matching optional magic filter. |
| `12` | `CMD_DEALS_GET` | Req → Resp | Query completed trade deal history matching time window and filters. |
| `13` | `CMD_SUBSCRIBE_TICKS` | Req → Resp | Subscribe to real-time pushed price quotes for symbol. |
| `14` | `CMD_UNSUBSCRIBE_TICKS`| Req → Resp | Unsubscribe from price quotes for symbol. |
| `15` | `CMD_SUBSCRIBE_TRADE` | Req → Resp | Subscribe to real-time broker trade transaction events (`OnTradeTransaction`). |
| `16` | `CMD_UNSUBSCRIBE_TRADE`| Req → Resp | Unsubscribe from trade transaction events. |
| `17` | `CMD_SUBSCRIBE_BOOK` | Req → Resp | Subscribe to Level II Market Depth DOM updates (`OnBookEvent`). |
| `18` | `CMD_UNSUBSCRIBE_BOOK` | Req → Resp | Unsubscribe from Level II Market Depth DOM updates. |
| `19` | `CMD_SUBSCRIBE_BARS` | Req → Resp | Subscribe to closed-bar notifications for symbol and timeframe. |
| `20` | `CMD_UNSUBSCRIBE_BARS`| Req → Resp | Unsubscribe from closed-bar notifications. |

---

## 4. Push Event Types (`kind = PKT_EVENT`)

| Event Type ID | Constant | Packed Struct | Size | Description |
| :---: | :--- | :--- | :---: | :--- |
| `1` | `EVENT_TICK` | `Mt5TickEvent` | 88 B | Real-time price quote emitted from `OnTick()`. |
| `2` | `EVENT_TRADE` | `Mt5TradeEvent` | 176 B | Trade transaction emitted from `OnTradeTransaction()`. |
| `3` | `EVENT_BOOK` | `Mt5BookEvent` | 72 B | Market Depth update emitted from `OnBookEvent()`. |
| `4` | `EVENT_BAR` | `Mt5BarEvent` | 96 B | Closed bar notification emitted on candle boundary. |

---

## 5. Authoritative Packed Struct Layouts (`#pragma pack(push, 1)`)

All structs are packed with 1-byte alignment. Struct sizes are compile-time asserted across C++, Rust, and MQL5.

### `PacketHdr` (16 bytes)
```c
typedef struct {
    uint32_t length;      // [0..3] Total packet size in bytes
    uint8_t  kind;        // [4]    0=Request, 1=Response, 2=Event
    uint8_t  _pad;        // [5]    Reserved padding
    uint16_t id;          // [6..7] Command ID or Event Type
    int32_t  status;      // [8..11] Status code (0 = success)
    uint32_t request_id;  // [12..15] Request correlation ID
} PacketHdr;
```

### `Mt5TickEvent` (88 bytes)
```c
typedef struct {
    char     symbol[32];  // [0..31]  Null-terminated symbol name
    int64_t  time_msc;    // [32..39] Timestamp in milliseconds (UTC)
    double   bid;         // [40..47] Current bid price
    double   ask;         // [48..55] Current ask price
    double   last;        // [56..63] Last trade price
    uint64_t volume;      // [64..71] Volume of last trade
    uint32_t flags;       // [72..75] Tick flags (TICK_FLAG_*)
    int32_t  _pad;        // [76..79] Alignment padding
    uint64_t event_seq;   // [80..87] Monotonic event sequence number
} Mt5TickEvent;
```

### `Mt5TradeEvent` (176 bytes)
```c
typedef struct {
    uint64_t deal;             // [0..7]   Deal ticket (0 if not a deal)
    uint64_t order;            // [8..15]  Order ticket
    uint64_t position;         // [16..23] Position ticket
    int64_t  time;             // [24..31] Execution timestamp in seconds (UTC)
    int32_t  trans_type;       // [32..35] MT5 ENUM_TRADE_TRANSACTION_TYPE
    int32_t  order_type;       // [36..39] MT5 ENUM_ORDER_TYPE
    double   price;            // [40..47] Execution price
    double   volume;           // [48..55] Executed volume
    double   sl;               // [56..63] Stop loss price
    double   tp;               // [64..71] Take profit price
    char     symbol[32];       // [72..103] Symbol name
    char     comment[32];      // [104..135] Trade comment
    int32_t  deal_entry;       // [136..139] MT5 ENUM_DEAL_ENTRY (0=In, 1=Out, etc.)
    int32_t  deal_type;        // [140..143] MT5 ENUM_DEAL_TYPE (0=Buy, 1=Sell)
    uint64_t deal_magic;       // [144..151] Magic number
    uint64_t deal_position_id; // [152..159] Position identifier
    int32_t  deal_reason;      // [160..163] MT5 ENUM_DEAL_REASON
    int32_t  _pad;             // [164..167] Alignment padding
    uint64_t event_seq;        // [168..175] Monotonic event sequence number
} Mt5TradeEvent;
```

### `Mt5BookEvent` (72 bytes)
```c
typedef struct {
    char     symbol[32];  // [0..31]  Symbol name
    int64_t  time_msc;    // [32..39] Millisecond timestamp
    int32_t  book_type;   // [40..43] 1 = Buy (Bid), 2 = Sell (Ask)
    int32_t  _pad;        // [44..47] Alignment padding
    double   price;       // [48..55] Price level
    double   volume;      // [56..63] Volume at price level
    uint64_t event_seq;   // [64..71] Monotonic event sequence number
} Mt5BookEvent;
```

### `Mt5BarEvent` (96 bytes)
```c
typedef struct {
    char     symbol[32];  // [0..31]  Symbol name
    int64_t  time;        // [32..39] Bar open timestamp in seconds (UTC)
    double   open;        // [40..47] Open price
    double   high;        // [48..55] High price
    double   low;         // [56..63] Low price
    double   close;       // [64..71] Close price
    int64_t  volume;      // [72..79] Bar volume
    int32_t  timeframe;   // [80..83] MT5 ENUM_TIMEFRAMES
    int32_t  spread;      // [84..87] Spread in points
    uint64_t event_seq;   // [88..95] Monotonic event sequence number
} Mt5BarEvent;
```

### `Mt5SymInfo` (76 bytes)
```c
typedef struct {
    double  point;       // [0..7]   Smallest price increment
    double  tick_value;  // [8..15]  Monetary value of 1 tick per 1.0 lot
    double  tick_size;   // [16..23] Minimum price tick size
    double  lot_step;    // [24..31] Lot step increment
    double  min_lot;     // [32..39] Minimum allowable volume
    double  max_lot;     // [40..47] Maximum allowable volume
    double  spread;      // [48..55] Current spread in points
    int32_t digits;      // [56..59] Decimal precision
    double  bid;         // [60..67] Latest terminal bid (Protocol v7+)
    double  ask;         // [68..75] Latest terminal ask (Protocol v7+)
} Mt5SymInfo;
```

### `Mt5Rate` (60 bytes)
```c
typedef struct {
    int64_t time;         // [0..7]   Bar open timestamp (UTC seconds)
    double  open;         // [8..15]  Open price
    double  high;         // [16..23] High price
    double  low;          // [24..31] Low price
    double  close;        // [32..39] Close price
    int64_t volume;       // [40..47] Tick volume
    int32_t spread;       // [48..51] Spread in points
    int64_t real_volume;  // [52..59] Real trade volume
} Mt5Rate;
```

### `Mt5Tick` (44 bytes)
```c
typedef struct {
    int64_t  time;    // [0..7]   Timestamp in seconds (UTC)
    double   bid;     // [8..15]  Bid price
    double   ask;     // [16..23] Ask price
    double   last;    // [24..31] Last deal price
    uint64_t volume;  // [32..39] Last deal volume
    uint32_t flags;   // [40..43] Tick flags
} Mt5Tick;
```

### `Mt5TradeResult` (44 bytes)
```c
typedef struct {
    uint32_t retcode;   // [0..3]   MT5 trade retcode (e.g. 10009 = TRADE_RETCODE_DONE)
    uint64_t deal;      // [4..11]  Deal ticket (0 if pending)
    uint64_t order;     // [12..19] Order ticket
    uint64_t position;  // [20..27] Position ticket (resolved via DEAL_POSITION_ID)
    double   volume;    // [28..35] Executed volume
    double   price;     // [36..43] Execution price
} Mt5TradeResult;
```

### `Mt5Position` (148 bytes)
```c
typedef struct {
    uint64_t ticket;         // [0..7]   Position ticket
    int64_t  time;           // [8..15]  Position open time (UTC seconds)
    int32_t  position_type;  // [16..19] 0 = Buy, 1 = Sell
    uint64_t magic;          // [20..27] Magic number
    double   volume;         // [28..35] Current volume
    double   price_open;     // [36..43] Open price
    double   sl;             // [44..51] Stop Loss price
    double   tp;             // [52..59] Take Profit price
    double   price_current;  // [60..67] Current market price
    double   profit;         // [68..75] Current floating profit
    double   swap;           // [76..83] Cumulative swap
    char     symbol[32];     // [84..115] Symbol name
    char     comment[32];    // [116..147] Position comment
} Mt5Position;
```

### `Mt5Order` (140 bytes)
```c
typedef struct {
    uint64_t ticket;          // [0..7]   Order ticket
    int64_t  time_setup;      // [8..15]  Setup time (UTC seconds)
    int32_t  order_type;      // [16..19] ENUM_ORDER_TYPE (BuyLimit, etc.)
    uint64_t magic;           // [20..27] Magic number
    double   volume_initial;  // [28..35] Requested initial volume
    double   volume_current;  // [36..43] Remaining active volume
    double   price_open;      // [44..51] Desired trigger price
    double   sl;              // [52..59] Stop Loss price
    double   tp;              // [60..67] Take Profit price
    double   price_current;   // [68..75] Current market price
    char     symbol[32];      // [76..107] Symbol name
    char     comment[32];     // [108..139] Order comment
} Mt5Order;
```

### `Mt5Deal` (152 bytes)
```c
typedef struct {
    uint64_t ticket;       // [0..7]   Deal ticket
    uint64_t order;        // [8..15]  Order ticket that generated this deal
    uint64_t position_id;  // [16..23] Position identifier
    int64_t  time;         // [24..31] Deal execution timestamp (UTC seconds)
    int32_t  deal_type;    // [32..35] 0 = Buy, 1 = Sell
    int32_t  entry;        // [36..39] 0 = In, 1 = Out, 2 = InOut, 3 = OutBy
    uint64_t magic;        // [40..47] Magic number
    double   volume;       // [48..55] Deal volume
    double   price;        // [56..63] Execution price
    double   commission;   // [64..71] Broker commission
    double   swap;         // [72..79] Overnight swap
    double   profit;       // [80..87] Closed trade profit
    char     symbol[32];   // [88..119] Symbol name
    char     comment[32];  // [120..151] Deal comment
} Mt5Deal;
```

---

## 6. Client Order ID & Wire Tokens

To achieve idempotent order submissions without exceeding the 31-character limit on MT5 order comments:

- **Token Format:** `w:<CrockfordBase32(10 chars)>` or `w:<12-char Crockford token>`.
- **Character Set:** `0-9` and uppercase `A-Z` excluding `I`, `L`, `O`, `U`.
- **Attribution:** The EA embeds the wire token into the order comment. On fills, partial executions, and closes, the token propagates to MT5 deal history and `OnTradeTransaction()`, allowing zero-latency attribution of broker events to the client order manager.

---

## 7. Buffer Limits & Constraints

| Parameter | Value | Location | Description |
| :--- | :---: | :--- | :--- |
| `MAX_FRAME_BYTES` | 1,048,576 B (1 MB) | DLL & EA | Upper bound on single framed packet size. |
| `MAX_EVENT_QUEUE` | 8,192 | DLL | Ring buffer capacity for push events before dropping oldest unread. |
| `TICK_QUEUE_CAPACITY` | 2,048 | EA | Internal tick queue capacity. |
| `TRADE_QUEUE_CAPACITY` | 512 | EA | Internal trade transaction queue capacity. |
| `InpTimerIntervalMs` | 1 ms | EA default | EA timer loop polling frequency. |
| `InpMaxRequestsPerTimer` | 64 | EA default | Maximum requests drained per timer tick. |
| `InpMaxTimerBudgetUs` | 5,000 µs | EA default | Execution time cap per timer tick to prevent UI starvation. |
| `InpEnforceMagicNumber` | `true` | EA default | Rejects trades where magic does not match EA configuration. |

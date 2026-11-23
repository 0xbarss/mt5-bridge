/* mt5_bridge.h
 * Stable public C API header for the MetaTrader 5 bridge.
 *
 * Language bindings (Python, C#, Go, etc.) include this header to call the
 * mt5_bridge.dll exported functions over the named-pipe IPC to the
 * mt5_bridge.mq5 Expert Advisor running inside MetaTrader 5.
 *
 * Struct layouts must match src/ffi.rs exactly (#[repr(C, packed)]). The
 * binary wire format and command IDs are versioned via
 * MT5_BRIDGE_PROTOCOL_VERSION; any layout or command change bumps it.
 */
#pragma once
#include <stdint.h>

#if defined(_WIN32) || defined(__CYGWIN__)
  #ifdef MT5_BRIDGE_EXPORTS
    #define MT5_BRIDGE_API __declspec(dllexport)
  #else
    #define MT5_BRIDGE_API __declspec(dllimport)
  #endif
#else
  #define MT5_BRIDGE_API __attribute__((visibility("default")))
#endif

/* Match #[repr(C, packed)] structs in src/ffi.rs — all packed, no padding. */
#pragma pack(push, 1)

typedef struct {
    double   point;
    double   tick_value;
    double   tick_size;
    double   lot_step;
    double   min_lot;
    double   max_lot;
    double   spread;
    int32_t  digits;
    double   bid;
    double   ask;
} Mt5SymInfo;            /* 76 bytes */

typedef struct {
    int64_t  time;
    double   open;
    double   high;
    double   low;
    double   close;
    int64_t  volume;
    int32_t  spread;
    int64_t  _rv;        /* real_volume — kept for ABI compat with MqlRates */
} Mt5Rate;               /* 60 bytes */

typedef struct {
    int64_t  time;
    double   bid;
    double   ask;
    double   last;
    uint64_t volume;
    uint32_t flags;
} Mt5Tick;               /* 44 bytes */

typedef struct {
    uint32_t retcode;
    uint64_t deal;
    uint64_t order;
    uint64_t position;
    double   volume;
    double   price;
} Mt5TradeResult;        /* 44 bytes */

typedef struct {
    uint64_t ticket;
    int64_t  time;
    int32_t  type;          /* 0 = Buy, 1 = Sell */
    uint64_t magic;
    double   volume;
    double   price_open;
    double   sl;
    double   tp;
    double   price_current;
    double   profit;
    double   swap;
    char     symbol[32];
    char     comment[32];
} Mt5Position;           /* 148 bytes */

typedef struct {
    uint64_t ticket;
    int64_t  time_setup;
    int32_t  type;          /* 2 = BuyLimit, 3 = SellLimit, 4 = BuyStop, 5 = SellStop */
    uint64_t magic;
    double   volume_initial;
    double   volume_current;
    double   price_open;
    double   sl;
    double   tp;
    double   price_current;
    char     symbol[32];
    char     comment[32];
} Mt5Order;              /* 140 bytes */

/* Completed trade deal from MT5 history (protocol v4+). */
typedef struct {
    uint64_t ticket;
    uint64_t order;         /* ticket of the order that produced this deal */
    uint64_t position_id;   /* DEAL_POSITION_ID */
    int64_t  time;          /* UTC seconds when the EA is configured with InpConvertToUTC */
    int32_t  type;          /* 0 = Buy, 1 = Sell */
    int32_t  entry;         /* 0 = In, 1 = Out, 2 = InOut, 3 = OutBy */
    uint64_t magic;
    double   volume;
    double   price;
    double   commission;
    double   swap;
    double   profit;
    char     symbol[32];
    char     comment[32];
} Mt5Deal;               /* 152 bytes */

/* Packet header for wire framing (protocol v6+). */
typedef struct {
    uint32_t length;     /* payload length in bytes */
    uint8_t  kind;       /* 0 = REQUEST, 1 = RESPONSE, 2 = EVENT */
    uint8_t  _pad;       /* 0 (alignment) */
    uint16_t id;         /* cmd for REQUEST/RESPONSE, event_type for EVENT */
    int32_t  status;     /* response status (>= 0 ok/count, < 0 error; 0 for req/event) */
    uint32_t request_id; /* request ID for matching responses; 0 for EVENT */
} PacketHdr;             /* 16 bytes */

/* Asynchronous market data tick event pushed by the EA (protocol v6+). */
typedef struct {
    char     symbol[32]; /* symbol name */
    int64_t  time_msc;   /* quote timestamp in milliseconds UTC */
    double   bid;
    double   ask;
    double   last;
    uint64_t volume;
    uint32_t flags;
    int32_t  _pad;
    uint64_t event_seq;  /* monotonic event sequence number */
} Mt5TickEvent;          /* 88 bytes */

/* Asynchronous trade transaction event pushed by the EA (protocol v6+). */
typedef struct {
    uint64_t deal;
    uint64_t order;
    uint64_t position;
    int64_t  time;
    int32_t  trans_type;      /* WTX_* wire constant */
    int32_t  order_type;
    double   price;
    double   volume;
    double   sl;
    double   tp;
    char     symbol[32];
    char     comment[32];
    int32_t  deal_entry;      /* 0 = In, 1 = Out, 2 = InOut, 3 = OutBy */
    int32_t  deal_type;       /* 0 = Buy, 1 = Sell */
    uint64_t deal_magic;
    uint64_t deal_position_id;
    int32_t  deal_reason;
    int32_t  _pad;
    uint64_t event_seq;       /* monotonic event sequence number */
} Mt5TradeEvent;              /* 176 bytes */

/* Asynchronous depth-of-market book event pushed by the EA (protocol v6+). */
typedef struct {
    char     symbol[32];
    int64_t  time_msc;
    int32_t  book_type;  /* 1 = BookBuy, 2 = BookSell */
    int32_t  _pad;
    double   price;
    double   volume;
    uint64_t event_seq;  /* monotonic event sequence number */
} Mt5BookEvent;          /* 72 bytes */

/* Asynchronous bar event pushed by the EA upon bar close (protocol v7+). */
typedef struct {
    char     symbol[32];
    int64_t  time;
    double   open;
    double   high;
    double   low;
    double   close;
    int64_t  volume;
    int32_t  timeframe;
    int32_t  spread;
    uint64_t event_seq;
} Mt5BarEvent;           /* 96 bytes */

/* Bridge-wide statistics snapshot (64 bytes, packed, protocol v9+).
 * Returned by CMD_STATS (27). Mirrors src/ffi.rs Mt5Stats. */
typedef struct {
    uint64_t uptime_secs;
    uint64_t requests_total;
    uint64_t tick_drops;
    uint64_t trade_drops;
    uint64_t book_drops;
    uint64_t bar_drops;
    uint32_t tick_queue_depth;
    uint32_t trade_queue_depth;
    int32_t  last_error;
    uint32_t protocol;
} Mt5Stats;               /* 64 bytes */

#pragma pack(pop)

/* Protocol version for wire handshake compatibility checks. */
#define MT5_BRIDGE_PROTOCOL_VERSION 8

/* Wire transaction kinds (WTX_*) - stable across MT5 builds */
#define MT5_BRIDGE_WTX_OTHER         0
#define MT5_BRIDGE_WTX_DEAL_ADD      1
#define MT5_BRIDGE_WTX_ORDER_ADD     2
#define MT5_BRIDGE_WTX_ORDER_DELETE  3
#define MT5_BRIDGE_WTX_ORDER_UPDATE  4
#define MT5_BRIDGE_WTX_POSITION      5
#define MT5_BRIDGE_WTX_REQUEST       6

/* Frame and query caps */
#define MT5_BRIDGE_MAX_FRAME_BYTES     (16 * 1024 * 1024) /* 16 MiB */
#define MT5_BRIDGE_MAX_BARS_PER_FETCH  ((MT5_BRIDGE_MAX_FRAME_BYTES - 64) / sizeof(Mt5Rate)) /* 279,620 */

/* Standard bridge operation return codes */
#define MT5_BRIDGE_OK                      1
#define MT5_BRIDGE_ERR_GENERAL             0
#define MT5_BRIDGE_ERR_SEND_FAILED        -1   /* Failed before transmission to MT5 (safe to retry) */
#define MT5_BRIDGE_ERR_UNKNOWN_EXECUTION  -2   /* Packet sent, but response lost/disconnected (uncertain execution - reconcile) */
#define MT5_BRIDGE_ERR_PIPE_DISCONNECTED  -3   /* Pipe not connected */
#define MT5_BRIDGE_ERR_SERVER_AUTH_FAILED -4   /* Server authentication failed */
#define MT5_BRIDGE_ERR_EA_REJECTED        -5   /* EA responded with an error status */
#define MT5_BRIDGE_ERR_ALREADY_INITIALIZED -6  /* Bridge client already initialized in this process */
#define MT5_BRIDGE_ERR_BUFFER_TOO_SMALL   -7   /* PollEvent: out_buf too small; *out_len reports the required size, nothing is copied, event stays queued */

#ifdef __cplusplus
extern "C" {
#endif

/* Connect to MT5 EA pipe server. Returns 1 on success, 0 on failure. */
MT5_BRIDGE_API int Initialize(int64_t login, const char* password, const char* server);

/* Disconnect and clean up. */
MT5_BRIDGE_API int Shutdown(void);

/* Check connection liveness. Returns 1 if connected and reader alive, 0 otherwise. */
MT5_BRIDGE_API int IsConnected(void);

/* Fetch OHLCV bars in [from, to] UTC range into buf (up to buf_capacity items).
   Returns bar count filled, -1 on error. */
MT5_BRIDGE_API int CopyRates(const char* symbol, int timeframe,
                             int64_t from, int64_t to,
                             Mt5Rate* buf, int buf_capacity);

/* Account balances, equity, margin, free margin, leverage, deposit currency.
   `currency` is a NUL-terminated buffer of at least 32 bytes. Returns 1 on success. */
MT5_BRIDGE_API int AccountInfo(double* balance, double* equity,
                               double* margin, double* free_margin,
                               double* leverage, char* currency, int currency_len);

/* Latest tick for a symbol. Returns 1 on success. */
MT5_BRIDGE_API int SymbolInfoTick(const char* symbol, Mt5Tick* tick);

/* Symbol properties (point size, tick value, tick size, lot limits, digits).
   Returns 1 on success. */
MT5_BRIDGE_API int SymbolInfoFull(const char* symbol, Mt5SymInfo* info);

/* Open or place an order. Returns 1 on success, -1 on send fail, -2 on unknown execution. */
MT5_BRIDGE_API int OrderSend(const char* symbol, int type, double volume,
                             double price, double sl, double tp,
                             const char* comment, uint32_t deviation,
                             int64_t expiration, uint64_t magic,
                             Mt5TradeResult* result);

/* Close position by ticket. Returns 1 on success, -1 on send fail, -2 on unknown execution. */
MT5_BRIDGE_API int OrderClose(uint64_t ticket, Mt5TradeResult* result);

/* Close position by ticket with magic number verification. */
MT5_BRIDGE_API int OrderCloseWithMagic(uint64_t ticket, uint64_t magic,
                                       Mt5TradeResult* result);

/* Close position by ticket with deviation, volume, and magic number. */
MT5_BRIDGE_API int OrderCloseWithDeviation(uint64_t ticket, uint64_t magic,
                                           uint32_t deviation, double volume,
                                           Mt5TradeResult* result);

/* Modify SL/TP with explicit level modes (0=keep, 1=clear, 2=set). */
MT5_BRIDGE_API int OrderModify(uint64_t ticket, uint8_t sl_mode, double sl,
                               uint8_t tp_mode, double tp,
                               Mt5TradeResult* result);

/* Modify SL/TP with magic number verification and explicit level modes. */
MT5_BRIDGE_API int OrderModifyWithMagic(uint64_t ticket, uint64_t magic,
                                        uint8_t sl_mode, double sl,
                                        uint8_t tp_mode, double tp,
                                        Mt5TradeResult* result);

/* Query open positions. Returns count filled into buf, or -1 on error. */
MT5_BRIDGE_API int PositionsGet(Mt5Position* buf, int buf_capacity,
                                uint64_t magic_filter, const char* symbol_filter);

/* Query pending orders. Returns count filled into buf, or -1 on error. */
MT5_BRIDGE_API int OrdersGet(Mt5Order* buf, int buf_capacity,
                             uint64_t magic_filter, const char* symbol_filter);

/* Query completed trade deals with from <= time <= to (UTC seconds).
   magic_filter 0 = any, symbol_filter NULL/"" = any. Returns count filled into buf,
   or -1 on error. */
MT5_BRIDGE_API int DealsGet(Mt5Deal* buf, int buf_capacity,
                            int64_t from, int64_t to,
                            uint64_t magic_filter, const char* symbol_filter);

/* Subscription management (protocol v6+) */
MT5_BRIDGE_API int SubscribeTicks(const char* symbol);
MT5_BRIDGE_API int UnsubscribeTicks(const char* symbol);
MT5_BRIDGE_API int SubscribeTrade(void);
MT5_BRIDGE_API int UnsubscribeTrade(void);
MT5_BRIDGE_API int SubscribeBook(const char* symbol);
MT5_BRIDGE_API int UnsubscribeBook(const char* symbol);
MT5_BRIDGE_API int SubscribeBars(const char* symbol, int timeframe);
MT5_BRIDGE_API int UnsubscribeBars(const char* symbol, int timeframe);

/* Heartbeat ping returning MT5 server time, monotonic microsecond counter,
   and negotiated protocol version. */
MT5_BRIDGE_API int Ping(int64_t* server_time_secs,
                        uint64_t* monotonic_microseconds,
                        uint32_t* protocol);

/* Last request DLL-side timing (microseconds since DLL load). */
MT5_BRIDGE_API void LastRequestTimings(uint64_t* write_start_us,
                                       uint64_t* write_end_us,
                                       uint64_t* response_at_us);

/* Symbol universe discovery (protocol v8+). */
MT5_BRIDGE_API int SymbolsTotal(void);
MT5_BRIDGE_API int SymbolsGet(uint32_t page, uint32_t page_size,
                              char* out_buf, int out_buf_capacity);
MT5_BRIDGE_API int SymbolSelect(const char* symbol, int enable);

/* Economic calendar (protocol v9+). */
MT5_BRIDGE_API int CalendarEvents(uint64_t from, uint64_t to,
                                  char* out_buf, int out_buf_capacity);
MT5_BRIDGE_API int CalendarCountries(char* out_buf, int out_buf_capacity);

/* Bridge statistics (protocol v9+).
   Stats packs an Mt5Stats struct into out_buf and returns 1 on success,
   -1 on failure. */
MT5_BRIDGE_API int Stats(char* out_buf, int out_buf_capacity);

/* Event callback registration and polling (protocol v6+) */
typedef void (*Mt5EventCallback)(uint16_t event_type, const void* payload,
                                 uint32_t payload_len);
MT5_BRIDGE_API int RegisterEventCallback(Mt5EventCallback cb);
MT5_BRIDGE_API int PollEvent(uint16_t* out_event_type, void* out_buf,
                             uint32_t buf_cap, uint32_t* out_len,
                             uint32_t timeout_ms);

/* Queue observability */
MT5_BRIDGE_API uint64_t EventsDroppedTotal(void);
MT5_BRIDGE_API uint64_t GetOrphanCount(void);

#ifdef __cplusplus
}
#endif

#if defined(__cplusplus)
static_assert(sizeof(Mt5SymInfo) == 76, "Mt5SymInfo size must be exactly 76 bytes");
static_assert(sizeof(Mt5Rate) == 60, "Mt5Rate size must be exactly 60 bytes");
static_assert(sizeof(Mt5Tick) == 44, "Mt5Tick size must be exactly 44 bytes");
static_assert(sizeof(Mt5TradeResult) == 44, "Mt5TradeResult size must be exactly 44 bytes");
static_assert(sizeof(Mt5Position) == 148, "Mt5Position size must be exactly 148 bytes");
static_assert(sizeof(Mt5Order) == 140, "Mt5Order size must be exactly 140 bytes");
static_assert(sizeof(Mt5Deal) == 152, "Mt5Deal size must be exactly 152 bytes");
static_assert(sizeof(PacketHdr) == 16, "PacketHdr size must be exactly 16 bytes");
static_assert(sizeof(Mt5TickEvent) == 88, "Mt5TickEvent size must be exactly 88 bytes");
static_assert(sizeof(Mt5TradeEvent) == 176, "Mt5TradeEvent size must be exactly 176 bytes");
static_assert(sizeof(Mt5BookEvent) == 72, "Mt5BookEvent size must be exactly 72 bytes");
static_assert(sizeof(Mt5BarEvent) == 96, "Mt5BarEvent size must be exactly 96 bytes");
static_assert(sizeof(Mt5Stats) == 64, "Mt5Stats size must be exactly 64 bytes");
#endif

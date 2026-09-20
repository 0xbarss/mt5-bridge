/* mt5_bridge.h
 * C header for the named-pipe bridge DLL.
 * Rust loads this DLL via libloading and calls the exported functions.
 * Struct layout must match src/ffi.rs exactly.
 */
#pragma once
#include <stdint.h>

#ifdef MT5_BRIDGE_EXPORTS
  #define MT5_API __declspec(dllexport)
#else
  #define MT5_API __declspec(dllimport)
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
} Mt5SymInfo;            /* 60 bytes */

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

#pragma pack(pop)

/* Protocol version for wire handshake compatibility checks */
#define MT5_BRIDGE_PROTOCOL_VERSION 3

/* Standard bridge operation return codes */
#define MT5_OK                     1
#define MT5_ERR_GENERAL            0
#define MT5_ERR_SEND_FAILED       -1   /* Failed before transmission to MT5 (safe to retry) */
#define MT5_ERR_UNKNOWN_EXECUTION -2   /* Packet sent, but response lost/disconnected (uncertain execution - reconcile) */
#define MT5_ERR_PIPE_DISCONNECTED -3   /* Pipe not connected */

#if defined(__cplusplus)
static_assert(sizeof(Mt5SymInfo) == 60, "Mt5SymInfo size must be exactly 60 bytes");
static_assert(sizeof(Mt5Rate) == 60, "Mt5Rate size must be exactly 60 bytes");
static_assert(sizeof(Mt5Tick) == 44, "Mt5Tick size must be exactly 44 bytes");
static_assert(sizeof(Mt5TradeResult) == 44, "Mt5TradeResult size must be exactly 44 bytes");
static_assert(sizeof(Mt5Position) == 148, "Mt5Position size must be exactly 148 bytes");
static_assert(sizeof(Mt5Order) == 140, "Mt5Order size must be exactly 140 bytes");
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* Connect to MT5 EA pipe server. Returns 1 on success, 0 on failure. */
MT5_API int Initialize(int64_t login, const char* password, const char* server);

/* Disconnect and clean up. */
MT5_API int Shutdown(void);

/* Fetch OHLCV bars in [from, to] UTC range into buf (up to buf_capacity items). Returns bar count filled, -1 on error. */
MT5_API int CopyRates(const char* symbol, int timeframe, int64_t from,
                      int64_t to, Mt5Rate* buf, int buf_capacity);

/* Account balances. Returns 1 on success. */
MT5_API int AccountInfo(double* balance, double* equity,
                        double* margin,  double* free_margin);

/* Open or place an order. Returns 1 on success, -1 on send fail, -2 on unknown execution. */
MT5_API int OrderSend(const char* symbol, int type, double volume,
                      double price, double sl, double tp,
                      const char* comment, uint32_t deviation,
                      int64_t expiration, uint64_t magic,
                      Mt5TradeResult* result);

/* Close position by ticket. Returns 1 on success, -1 on send fail, -2 on unknown execution. */
MT5_API int OrderClose(uint64_t ticket, Mt5TradeResult* result);

/* Close position by ticket with magic number verification. Returns 1 on success, -1 on send fail, -2 on unknown execution. */
MT5_API int OrderCloseWithMagic(uint64_t ticket, uint64_t magic, Mt5TradeResult* result);

/* Modify SL/TP. Returns 1 on success, -1 on send fail, -2 on unknown execution. */
MT5_API int OrderModify(uint64_t ticket, double sl, double tp, Mt5TradeResult* result);

/* Modify SL/TP with magic number verification. Returns 1 on success, -1 on send fail, -2 on unknown execution. */
MT5_API int OrderModifyWithMagic(uint64_t ticket, uint64_t magic, double sl, double tp, Mt5TradeResult* result);

/* Latest tick for a symbol. Returns 1 on success. */
MT5_API int SymbolInfoTick(const char* symbol, Mt5Tick* tick);

/* Symbol properties (point size, tick value, tick size, lot limits, digits). Returns 1 on success. */
MT5_API int SymbolInfoFull(const char* symbol, Mt5SymInfo* info);

/* Query open positions. Returns count filled into buf, or -1 on error. */
MT5_API int PositionsGet(Mt5Position* buf, int buf_capacity, uint64_t magic_filter, const char* symbol_filter);

/* Query pending orders. Returns count filled into buf, or -1 on error. */
MT5_API int OrdersGet(Mt5Order* buf, int buf_capacity, uint64_t magic_filter, const char* symbol_filter);

#ifdef __cplusplus
}
#endif

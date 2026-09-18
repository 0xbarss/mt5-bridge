/* mt5_bridge.h
 * C header for the named-pipe bridge DLL.
 * Rust loads this DLL via libloading and calls the exported functions.
 * Struct layout must match crates/broker/src/mt5/mod.rs exactly.
 */
#pragma once
#include <stdint.h>

#ifdef MT5_BRIDGE_EXPORTS
  #define MT5_API __declspec(dllexport)
#else
  #define MT5_API __declspec(dllimport)
#endif

/* Match #[repr(C)] structs in mod.rs — all packed, no padding. */
#pragma pack(push, 1)

typedef struct {
    double   point;
    double   tick_value;
    double   lot_step;
    double   min_lot;
    double   max_lot;
    double   spread;
    int32_t  digits;
} Mt5SymInfo;            /* 52 bytes */

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
    double   volume;
    double   price;
} Mt5TradeResult;        /* 36 bytes */

#pragma pack(pop)

#ifdef __cplusplus
extern "C" {
#endif

/* Connect to MT5 EA pipe server. Returns 1 on success, 0 on failure. */
MT5_API int Initialize(long login, const char* password, const char* server);

/* Disconnect and clean up. */
MT5_API int Shutdown(void);

/* Fetch OHLCV bars in [from, to] UTC range. Returns bar count filled, -1 on error. */
MT5_API int CopyRates(const char* symbol, int timeframe, int64_t from,
                      int64_t to, Mt5Rate* buf);

/* Account balances. Returns 1 on success. */
MT5_API int AccountInfo(double* balance, double* equity,
                        double* margin,  double* free_margin);

/* Open a market order. Returns 1 on success. */
MT5_API int OrderSend(const char* symbol, int type, double volume,
                      double price, double sl, double tp,
                      const char* comment, Mt5TradeResult* result);

/* Close position by ticket. Returns 1 on success. */
MT5_API int OrderClose(uint64_t ticket, Mt5TradeResult* result);

/* Modify SL/TP. Returns 1 on success. */
MT5_API int OrderModify(uint64_t ticket, double sl, double tp);

/* Latest tick for a symbol. Returns 1 on success. */
MT5_API int SymbolInfoTick(const char* symbol, Mt5Tick* tick);

/* Symbol properties (point size, tick value, lot limits, digits). Returns 1 on success. */
MT5_API int SymbolInfoFull(const char* symbol, Mt5SymInfo* info);

#ifdef __cplusplus
}
#endif

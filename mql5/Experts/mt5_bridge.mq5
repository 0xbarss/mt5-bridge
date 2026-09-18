//+------------------------------------------------------------------+
//| mt5_bridge.mq5                                                   |
//| Named-pipe SERVER for the Rust trading engine.                   |
//|                                                                  |
//| Deploy:                                                          |
//|   Copy to  MQL5/Experts/mt5_bridge.mq5  and compile in MT5.     |
//|   Attach to any chart. Enable "Allow algorithmic trading".       |
//|   Settings → Options → Expert Advisors → "Allow DLL imports".   |
//|                                                                  |
//| Protocol (binary, little-endian):                                |
//|   Request  : [uint32 cmd][uint32 payload_len][payload]           |
//|   Response : [int32  status][uint32 data_len][data]              |
//|   status ≥ 0 → ok  (CopyRates: bar count; others: 1)            |
//|   status < 0 → error                                             |
//|   Strings: [uint32 len][bytes] — ASCII, no NUL                   |
//+------------------------------------------------------------------+
#property strict
#property version   "1.0"
#property description "MT5 Bridge — named pipe server for Rust trading engine"

//---- Require DLL imports (kernel32) to be enabled in MT5 settings.
#import "kernel32.dll"
long  CreateNamedPipeW(string name,
                       uint   dwOpenMode,
                       uint   dwPipeMode,
                       uint   nMaxInstances,
                       uint   nOutBufferSize,
                       uint   nInBufferSize,
                       uint   nDefaultTimeOut,
                       long   lpSecurityAttributes);
bool  ConnectNamedPipe(long hPipe, long lpOverlapped);
bool  DisconnectNamedPipe(long hPipe);
bool  ReadFile(long hFile, uchar &buf[], uint toRead,
               uint &bytesRead, long lpOverlapped);
bool  WriteFile(long hFile, const uchar &buf[], uint toWrite,
                uint &bytesWritten, long lpOverlapped);
bool  CloseHandle(long hObject);
bool  FlushFileBuffers(long hFile);
bool  PeekNamedPipe(long hPipe, uchar &buf[], uint bufSize,
                    uint &bytesRead, uint &totalAvail,
                    uint &bytesLeft);
// NOTE: GetLastError() is intentionally NOT imported — MQL5's built-in function
// of the same name shadows any kernel32 import, always returning MQL5 error
// codes (0 = no error) instead of Windows system error codes.
bool  SetNamedPipeHandleState(long hPipe, uint &lpMode,
                              long lpMaxCollectionCount,
                              long lpCollectDataTimeout);
#import

//---- Constants
#define PIPE_ACCESS_DUPLEX      3
#define PIPE_TYPE_BYTE          0
#define PIPE_READMODE_BYTE      0
#define PIPE_WAIT               0
#define PIPE_NOWAIT             1
#define PIPE_UNLIMITED_INSTANCES 255
// INVALID_HANDLE is a built-in MQL5 constant (-1); no redefinition needed.
#define ERROR_BROKEN_PIPE       109
#define TIMER_INTERVAL_MS       50

//---- Protocol commands
#define CMD_INIT         1
#define CMD_SHUTDOWN     2
#define CMD_RATES        3
#define CMD_ACCOUNT      4
#define CMD_ORDER_SEND   5
#define CMD_ORDER_CLOSE  6
#define CMD_ORDER_MODIFY 7
#define CMD_SYM_TICK     8
#define CMD_SYM_INFO     9

//---- EA input
input int InpMagicNumber = 20240101;  // Magic number for bridge orders
input string InpPipeName = "mt5bridge"; // Custom Named Pipe Name

//---- State
long g_server  = INVALID_HANDLE;
long g_client  = INVALID_HANDLE;
bool g_running = false;
string g_pipe_path = "";

// ── I/O helpers ─────────────────────────────────────────────────────────────

bool PipeReadExact(long h, uchar &out[], uint need) {
    ArrayResize(out, need);
    uint offset = 0;
    while (offset < need) {
        uchar chunk[];
        ArrayResize(chunk, need - offset);
        uint got = 0;
        if (!ReadFile(h, chunk, need - offset, got, 0) || got == 0) return false;
        ArrayCopy(out, chunk, offset, 0, got);
        offset += got;
    }
    return true;
}

bool PipeWriteExact(long h, const uchar &data[], uint len) {
    uint offset = 0;
    while (offset < len) {
        uint chunk_len = len - offset;
        uchar chunk[];
        ArrayResize(chunk, chunk_len);
        ArrayCopy(chunk, data, 0, offset, chunk_len);
        uint written = 0;
        if (!WriteFile(h, chunk, chunk_len, written, 0) || written == 0)
            return false;
        offset += written;
    }
    return true;
}

// ── Binary unpack helpers ────────────────────────────────────────────────────

uint UnpackU32(const uchar &b[], int off) {
    return (uint)b[off]
         | ((uint)b[off+1] << 8)
         | ((uint)b[off+2] << 16)
         | ((uint)b[off+3] << 24);
}

int UnpackI32(const uchar &b[], int off) { return (int)UnpackU32(b, off); }

long UnpackI64(const uchar &b[], int off) {
    ulong lo = (ulong)UnpackU32(b, off);
    ulong hi = (ulong)UnpackU32(b, off + 4);
    return (long)(lo | (hi << 32));
}

ulong UnpackU64(const uchar &b[], int off) {
    ulong lo = (ulong)UnpackU32(b, off);
    ulong hi = (ulong)UnpackU32(b, off + 4);
    return lo | (hi << 32);
}

// Reinterpret 8 bytes as IEEE-754 double via a struct.
struct DoubleBytes { double v; };
double UnpackF64(const uchar &b[], int off) {
    uchar tmp[8];
    ArrayCopy(tmp, b, 0, off, 8);
    DoubleBytes db;
    CharArrayToStruct(db, tmp);
    return db.v;
}

// Unpack length-prefixed ASCII string.
string UnpackStr(const uchar &b[], int &off) {
    uint len = UnpackU32(b, off); off += 4;
    if (len == 0) return "";
    uchar tmp[];
    ArrayResize(tmp, len + 1);
    ArrayCopy(tmp, b, 0, off, len);
    tmp[len] = 0;
    off += len;
    return CharArrayToString(tmp, 0, len, CP_ACP);
}

// ── Binary pack helpers ──────────────────────────────────────────────────────

void PackI32(uchar &b[], int v) {
    int n = ArraySize(b); ArrayResize(b, n + 4);
    b[n]   = (uchar)(v & 0xFF);
    b[n+1] = (uchar)((v >> 8)  & 0xFF);
    b[n+2] = (uchar)((v >> 16) & 0xFF);
    b[n+3] = (uchar)((v >> 24) & 0xFF);
}

void PackU32(uchar &b[], uint v) {
    int n = ArraySize(b); ArrayResize(b, n + 4);
    b[n]   = (uchar)(v & 0xFF);
    b[n+1] = (uchar)((v >> 8)  & 0xFF);
    b[n+2] = (uchar)((v >> 16) & 0xFF);
    b[n+3] = (uchar)((v >> 24) & 0xFF);
}

void PackI64(uchar &b[], long v) {
    PackI32(b, (int)(v & 0xFFFFFFFF));
    PackI32(b, (int)((ulong)v >> 32));
}

void PackU64(uchar &b[], ulong v) { PackI64(b, (long)v); }

void PackF64(uchar &b[], double v) {
    DoubleBytes db; db.v = v;
    uchar tmp[8];
    StructToCharArray(db, tmp);
    int n = ArraySize(b); ArrayResize(b, n + 8);
    ArrayCopy(b, tmp, n, 0, 8);
}

// ── Response senders ─────────────────────────────────────────────────────────

bool SendResponse(long h, int status, const uchar &data[], uint data_len) {
    uchar hdr[8];
    // status (4 bytes)
    hdr[0] = (uchar)(status & 0xFF);
    hdr[1] = (uchar)((status >> 8)  & 0xFF);
    hdr[2] = (uchar)((status >> 16) & 0xFF);
    hdr[3] = (uchar)((status >> 24) & 0xFF);
    // data_len (4 bytes)
    hdr[4] = (uchar)(data_len & 0xFF);
    hdr[5] = (uchar)((data_len >> 8)  & 0xFF);
    hdr[6] = (uchar)((data_len >> 16) & 0xFF);
    hdr[7] = (uchar)((data_len >> 24) & 0xFF);
    if (!PipeWriteExact(h, hdr, 8)) return false;
    if (data_len > 0 && !PipeWriteExact(h, data, data_len)) return false;
    FlushFileBuffers(h);
    return true;
}

bool SendOk(long h) {
    uchar empty[1]; // unused
    return SendResponse(h, 1, empty, 0);
}

bool SendOkData(long h, const uchar &data[], uint len) {
    return SendResponse(h, (int)len > 0 ? 1 : 1, data, len);
}

bool SendCount(long h, int count, const uchar &data[], uint data_len) {
    return SendResponse(h, count, data, data_len);
}

bool SendError(long h) {
    uchar empty[1];
    return SendResponse(h, -1, empty, 0);
}

// ── Command handlers ──────────────────────────────────────────────────────────

void HandleInit(long h, const uchar &payload[], uint len) {
    // payload: i64 login + str password + str server
    // We already have the connection — just confirm credentials are plausible.
    // (The EA runs inside MT5 which is already logged in; we don't re-login here.)
    // If the EA's active account matches, respond OK.
    if (len < 8) { SendError(h); return; }
    int off = 8; // skip login i64 (we don't use it; MT5 is already connected)
    // Read password and server strings (ignored; MT5 manages its own session)
    // Just respond OK.
    SendOk(h);
    Print("MT5Bridge: client authenticated");
}

void HandleShutdown(long h) {
    SendOk(h);
    Print("MT5Bridge: client requested shutdown");
    // We'll detect client disconnect in the main loop.
}

void HandleCopyRates(long h, const uchar &payload[], uint len) {
    if (len < 4) { SendError(h); return; }
    int off = 0;
    string sym = UnpackStr(payload, off);
    if ((uint)off + 4 + 8 + 8 > len) { SendError(h); return; }
    int  tf   = UnpackI32(payload, off); off += 4;
    long from = UnpackI64(payload, off); off += 8;
    long to   = UnpackI64(payload, off); off += 8;

    long offset = (long)TimeTradeServer() - (long)TimeGMT();
    ENUM_TIMEFRAMES mtf     = (ENUM_TIMEFRAMES)tf;
    datetime        dt_from = (datetime)(from + offset);
    datetime        dt_to   = (datetime)(to + offset);

    MqlRates rates[];
    ArraySetAsSeries(rates, false);
    
    // Force MT5 to download history back to the requested start date if not locally cached
    datetime now = TimeTradeServer();
    if (dt_from < now) {
        int required_bars = (int)((now - dt_from) / PeriodSeconds(mtf)) + 100;
        if (required_bars > 0) {
            MqlRates dummy[];
            CopyRates(sym, mtf, 0, required_bars, dummy);
        }
    }

    ResetLastError();
    int filled = CopyRates(sym, mtf, dt_from, dt_to, rates);

    if (filled <= 0) { uchar e[1]; SendCount(h, 0, e, 0); return; }

    // Serialise MqlRates[] into Mt5Rate bytes (layouts match — 60 bytes each).
    uchar data[];
    ArrayResize(data, filled * 60);
    for (int i = 0; i < filled; i++) {
        rates[i].time = (datetime)((long)rates[i].time - offset);
        uchar tmp[];
        StructToCharArray(rates[i], tmp);
        ArrayCopy(data, tmp, i * 60, 0, 60);
    }
    SendCount(h, filled, data, (uint)(filled * 60));
}

void HandleAccount(long h) {
    double balance    = AccountInfoDouble(ACCOUNT_BALANCE);
    double equity     = AccountInfoDouble(ACCOUNT_EQUITY);
    double margin     = AccountInfoDouble(ACCOUNT_MARGIN);
    double free_margin= AccountInfoDouble(ACCOUNT_MARGIN_FREE);

    uchar data[];
    PackF64(data, balance);
    PackF64(data, equity);
    PackF64(data, margin);
    PackF64(data, free_margin);
    SendOkData(h, data, (uint)ArraySize(data));
}

// MQL5 has no #pragma pack. BridgeTradeResult is only used as a local
// intermediate; its wire bytes are written field-by-field via Pack* helpers.
struct BridgeTradeResult {
    uint   retcode;
    ulong  deal;
    ulong  order;
    double volume;
    double price;
};

void HandleOrderSend(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym     = UnpackStr(payload, off);
    if ((uint)off + 4 + 8*5 > len) { SendError(h); return; }
    int    otype   = UnpackI32(payload, off); off += 4;
    double volume  = UnpackF64(payload, off); off += 8;
    double price   = UnpackF64(payload, off); off += 8;
    double sl      = UnpackF64(payload, off); off += 8;
    double tp      = UnpackF64(payload, off); off += 8;
    string comment = UnpackStr(payload, off);

    ENUM_ORDER_TYPE order_type = (otype == 0) ? ORDER_TYPE_BUY : ORDER_TYPE_SELL;

    MqlTradeRequest req = {};
    req.action      = TRADE_ACTION_DEAL;
    req.symbol      = sym;
    req.volume      = volume;
    req.type        = order_type;
    req.price       = SymbolInfoDouble(sym, (otype == 0) ? SYMBOL_ASK : SYMBOL_BID);
    req.sl          = sl;
    req.tp          = tp;
    req.deviation   = 10;
    req.magic       = InpMagicNumber;
    req.comment     = comment;
    req.type_filling= ORDER_FILLING_IOC;

    MqlTradeResult res = {};
    bool ok = OrderSend(req, res);

    double exec_price = res.price;
    if (exec_price <= 0.0) {
        if (res.deal > 0 && HistoryDealSelect(res.deal)) {
            exec_price = HistoryDealGetDouble(res.deal, DEAL_PRICE);
        }
        if (exec_price <= 0.0 && res.order > 0 && PositionSelectByTicket(res.order)) {
            exec_price = PositionGetDouble(POSITION_PRICE_OPEN);
        }
        if (exec_price <= 0.0 && PositionSelect(sym)) {
            exec_price = PositionGetDouble(POSITION_PRICE_OPEN);
        }
        if (exec_price <= 0.0) {
            exec_price = req.price;
        }
    }

    // Serialise as packed wire format: u32 retcode + u64 deal + u64 order +
    // f64 volume + f64 price = 36 bytes (matches Mt5TradeResult in mt5_bridge.h).
    uchar data[];
    PackU32(data, res.retcode);
    PackU64(data, res.deal);
    PackU64(data, res.order);
    PackF64(data, res.volume);
    PackF64(data, exec_price);

    if (ok && (res.retcode == TRADE_RETCODE_DONE ||
               res.retcode == TRADE_RETCODE_PLACED)) {
        SendOkData(h, data, (uint)ArraySize(data));
    } else {
        Print("MT5Bridge: OrderSend failed retcode=", res.retcode);
        SendResponse(h, -1, data, (uint)ArraySize(data));
    }
}

void HandleOrderClose(long h, const uchar &payload[], uint len) {
    if (len < 8) { SendError(h); return; }
    ulong ticket = UnpackU64(payload, 0);

    // Find position by ticket and close it.
    if (!PositionSelectByTicket((ulong)ticket)) {
        Print("MT5Bridge: position not found for ticket=", ticket);
        SendError(h);
        return;
    }

    string  sym   = PositionGetString(POSITION_SYMBOL);
    double  vol   = PositionGetDouble(POSITION_VOLUME);
    ENUM_POSITION_TYPE ptype = (ENUM_POSITION_TYPE)PositionGetInteger(POSITION_TYPE);
    ENUM_ORDER_TYPE close_type = (ptype == POSITION_TYPE_BUY)
                                  ? ORDER_TYPE_SELL : ORDER_TYPE_BUY;

    MqlTradeRequest req = {};
    req.action   = TRADE_ACTION_DEAL;
    req.symbol   = sym;
    req.volume   = vol;
    req.type     = close_type;
    req.price    = SymbolInfoDouble(sym, (close_type == ORDER_TYPE_BUY)
                                        ? SYMBOL_ASK : SYMBOL_BID);
    req.deviation= 10;
    req.position = ticket;
    req.magic    = InpMagicNumber;
    req.comment  = "bridge_close";
    req.type_filling = ORDER_FILLING_IOC;

    MqlTradeResult res = {};
    bool ok = OrderSend(req, res);

    double exec_price = res.price;
    if (exec_price <= 0.0) {
        if (res.deal > 0 && HistoryDealSelect(res.deal)) {
            exec_price = HistoryDealGetDouble(res.deal, DEAL_PRICE);
        }
        if (exec_price <= 0.0) {
            exec_price = req.price;
        }
    }

    uchar data[];
    PackU32(data, res.retcode);
    PackU64(data, res.deal);
    PackU64(data, res.order);
    PackF64(data, res.volume);
    PackF64(data, exec_price);

    if (ok && (res.retcode == TRADE_RETCODE_DONE ||
               res.retcode == TRADE_RETCODE_PLACED)) {
        SendOkData(h, data, (uint)ArraySize(data));
    } else {
        Print("MT5Bridge: OrderClose failed retcode=", res.retcode);
        SendResponse(h, -1, data, (uint)ArraySize(data));
    }
}

void HandleOrderModify(long h, const uchar &payload[], uint len) {
    if (len < 8 + 16) { SendError(h); return; }
    ulong  ticket = UnpackU64(payload, 0);
    double sl     = UnpackF64(payload, 8);
    double tp     = UnpackF64(payload, 16);

    if (!PositionSelectByTicket(ticket)) { SendError(h); return; }
    string sym = PositionGetString(POSITION_SYMBOL);

    MqlTradeRequest req = {};
    req.action   = TRADE_ACTION_SLTP;
    req.symbol   = sym;
    req.position = ticket;
    req.sl       = sl;
    req.tp       = tp;
    req.magic    = InpMagicNumber;

    MqlTradeResult res = {};
    bool ok = OrderSend(req, res);

    if (ok && (res.retcode == TRADE_RETCODE_DONE ||
               res.retcode == TRADE_RETCODE_PLACED)) {
        SendOk(h);
    } else {
        Print("MT5Bridge: OrderModify failed retcode=", res.retcode);
        SendError(h);
    }
}

// MQL5 has no #pragma pack. These structs are local intermediates only;
// wire bytes are written field-by-field via Pack* helpers below.
struct BridgeTick {
    long   time;
    double bid;
    double ask;
    double last;
    ulong  volume;
    uint   flags;
};

struct BridgeSymInfo {
    double point;
    double tick_value;
    double lot_step;
    double min_lot;
    double max_lot;
    double spread;
    int    digits;
};

void HandleSymbolInfoFull(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = UnpackStr(payload, off);

    double point      = SymbolInfoDouble(sym, SYMBOL_POINT);
    double tick_value = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_VALUE);
    double lot_step   = SymbolInfoDouble(sym, SYMBOL_VOLUME_STEP);
    double min_lot    = SymbolInfoDouble(sym, SYMBOL_VOLUME_MIN);
    double max_lot    = SymbolInfoDouble(sym, SYMBOL_VOLUME_MAX);
    int    digits     = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);

    MqlTick tick;
    double spread = 0.0;
    if (SymbolInfoTick(sym, tick) && point > 0.0)
        spread = (tick.ask - tick.bid) / point;

    // Serialise as packed wire format: 6×f64 + i32 = 52 bytes
    // (matches Mt5SymInfo in mt5_bridge.h).
    uchar data[];
    PackF64(data, point);
    PackF64(data, tick_value);
    PackF64(data, lot_step);
    PackF64(data, min_lot);
    PackF64(data, max_lot);
    PackF64(data, spread);
    PackI32(data, digits);
    SendOkData(h, data, (uint)ArraySize(data));
}

void HandleSymbolInfoTick(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = UnpackStr(payload, off);

    MqlTick tick;
    if (!SymbolInfoTick(sym, tick)) { SendError(h); return; }

    long offset = (long)TimeTradeServer() - (long)TimeGMT();

    // Serialise as packed wire format: i64 + 3×f64 + u64 + u32 = 44 bytes
    // (matches Mt5Tick in mt5_bridge.h).
    uchar data[];
    PackI64(data, (long)tick.time - offset);
    PackF64(data, tick.bid);
    PackF64(data, tick.ask);
    PackF64(data, tick.last);
    PackU64(data, tick.volume);
    PackU32(data, tick.flags);
    SendOkData(h, data, (uint)ArraySize(data));
}

// ── Request dispatcher ────────────────────────────────────────────────────────

bool DispatchRequest(long h) {
    // Read 8-byte request header: [uint32 cmd][uint32 payload_len]
    uchar hdr[];
    if (!PipeReadExact(h, hdr, 8)) return false;

    uint cmd     = UnpackU32(hdr, 0);
    uint pay_len = UnpackU32(hdr, 4);

    uchar payload[];
    if (pay_len > 0) {
        if (!PipeReadExact(h, payload, pay_len)) return false;
    }

    switch (cmd) {
        case CMD_INIT:        HandleInit(h, payload, pay_len);              break;
        case CMD_SHUTDOWN:    HandleShutdown(h); return false;  /* disconnect */
        case CMD_RATES:       HandleCopyRates(h, payload, pay_len);         break;
        case CMD_ACCOUNT:     HandleAccount(h);                             break;
        case CMD_ORDER_SEND:  HandleOrderSend(h, payload, pay_len);         break;
        case CMD_ORDER_CLOSE: HandleOrderClose(h, payload, pay_len);        break;
        case CMD_ORDER_MODIFY:HandleOrderModify(h, payload, pay_len);       break;
        case CMD_SYM_TICK:    HandleSymbolInfoTick(h, payload, pay_len);    break;
        case CMD_SYM_INFO:    HandleSymbolInfoFull(h, payload, pay_len);    break;
        default:
            Print("MT5Bridge: unknown cmd=", cmd);
            SendError(h);
            break;
    }
    return true;
}

// ── EA lifecycle ──────────────────────────────────────────────────────────────

int OnInit() {
    g_pipe_path = "\\\\.\\pipe\\" + InpPipeName;
    // PIPE_NOWAIT: makes ConnectNamedPipe non-blocking so it doesn't freeze
    // OnTimer. Switched back to PIPE_WAIT via SetNamedPipeHandleState after
    // a client connects, so data I/O remains synchronous/reliable.
    g_server = CreateNamedPipeW(
        g_pipe_path,
        PIPE_ACCESS_DUPLEX,           // duplex
        PIPE_TYPE_BYTE | PIPE_NOWAIT, // byte-stream, non-blocking connect poll
        1,                            // 1 instance (single Rust client)
        65536,                        // out buffer
        65536,                        // in buffer
        0,                            // default timeout
        0                             // default security
    );

    if (g_server == INVALID_HANDLE) {
        // Note: GetLastError() here returns the MQL5 error code (not Windows)
        // due to MQL5 built-in shadowing kernel32's GetLastError.
        Print("MT5Bridge: CreateNamedPipe failed — check DLL imports are enabled and no other instance is running");
        return INIT_FAILED;
    }

    g_running = true;
    EventSetMillisecondTimer(TIMER_INTERVAL_MS);
    Print("MT5Bridge: pipe server ready — waiting for Rust client");
    return INIT_SUCCEEDED;
}

void OnDeinit(const int reason) {
    EventKillTimer();
    g_running = false;
    if (g_client != INVALID_HANDLE) {
        DisconnectNamedPipe(g_client);
        CloseHandle(g_client);
        g_client = INVALID_HANDLE;
    }
    if (g_server != INVALID_HANDLE) {
        CloseHandle(g_server);
        g_server = INVALID_HANDLE;
    }
    Print("MT5Bridge: shutdown (reason=", reason, ")");
}

void OnTimer() {
    if (!g_running || g_server == INVALID_HANDLE) return;

    // Poll for an incoming client connection without blocking the timer thread.
    if (g_client == INVALID_HANDLE) {
        // In PIPE_NOWAIT mode ConnectNamedPipe always returns FALSE.  We cannot
        // rely on GetLastError() here because MQL5's built-in shadows kernel32's
        // version and always returns 0.  Use PeekNamedPipe instead: it succeeds
        // only when a client is actually connected.
        ConnectNamedPipe(g_server, 0);
        uchar peek_conn[1];
        uint  peek_rd = 0, peek_av = 0, peek_lf = 0;
        if (PeekNamedPipe(g_server, peek_conn, 0, peek_rd, peek_av, peek_lf)) {
            g_client = g_server;
            // Switch to blocking mode for reliable synchronous I/O.
            uint mode = PIPE_READMODE_BYTE | PIPE_WAIT;
            SetNamedPipeHandleState(g_client, mode, 0, 0);
            Print("MT5Bridge: Rust client connected");
        }
        return;
    }

    // Service one request per timer tick (non-blocking check first).
    uchar peek_buf[1];
    uint  peek_read = 0, peek_avail = 0, peek_left = 0;
    bool has_data = PeekNamedPipe(g_client, peek_buf, 1,
                                  peek_read, peek_avail, peek_left);
    if (!has_data || peek_avail == 0) return;  // nothing to read yet

    bool ok = DispatchRequest(g_client);
    if (!ok) {
        Print("MT5Bridge: client disconnected");
        DisconnectNamedPipe(g_client);
        g_client = INVALID_HANDLE;

        // Close the old handle before recreating — avoids a handle leak.
        CloseHandle(g_server);
        g_server = CreateNamedPipeW(
            g_pipe_path,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_NOWAIT, // keep PIPE_NOWAIT for non-blocking connect poll
            1, 65536, 65536, 0, 0
        );
        if (g_server == INVALID_HANDLE)
            Print("MT5Bridge: failed to recreate pipe");
        else
            Print("MT5Bridge: re-listening for next Rust client");
    }
}

// Required by MT5 even when not used.
void OnTick() {}

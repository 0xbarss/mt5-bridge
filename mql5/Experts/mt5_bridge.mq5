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

//---- Win32 structures for security descriptors (explicit x64 8-byte alignment)
struct SECURITY_ATTRIBUTES {
    uint   nLength;
    uint   _padding;
    long   lpSecurityDescriptor;
    int    bInheritHandle;
    int    _padding2;
};

//---- Require DLL imports (advapi32, kernel32) to be enabled in MT5 settings.
#import "advapi32.dll"
bool ConvertStringSecurityDescriptorToSecurityDescriptorW(
    string  StringSecurityDescriptor,
    uint    StringSDRevision,
    long   &SecurityDescriptor,
    uint   &SecurityDescriptorSize
);
#import

#import "kernel32.dll"
long  CreateNamedPipeW(string name,
                       uint   dwOpenMode,
                       uint   dwPipeMode,
                       uint   nMaxInstances,
                       uint   nOutBufferSize,
                       uint   nInBufferSize,
                       uint   nDefaultTimeOut,
                       SECURITY_ATTRIBUTES &lpSecurityAttributes);
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
long  LocalFree(long hMem);
// Named event used to ask an already-running bridge instance to release the
// pipe so a freshly started instance (e.g. from the headless startup config)
// can take over with its new inputs.
long  CreateEventW(long lpEventAttributes, int bManualReset,
                   int bInitialState, string lpName);
bool  SetEvent(long hEvent);
bool  ResetEvent(long hEvent);
uint  WaitForSingleObject(long hHandle, uint dwMilliseconds);
#import

//---- Constants
#define PIPE_ACCESS_DUPLEX      3
#define PIPE_TYPE_BYTE          0
#define PIPE_READMODE_BYTE      0
#define PIPE_WAIT               0
#define PIPE_NOWAIT             1
#define PIPE_UNLIMITED_INSTANCES 255
#define PIPE_REJECT_REMOTE_CLIENTS 0x00000008
// INVALID_HANDLE is a built-in MQL5 constant (-1); no redefinition needed.
#define ERROR_BROKEN_PIPE       109
#define WAIT_OBJECT_0           0
#define TIMER_INTERVAL_MS       50
#define MAX_PAYLOAD_SIZE        16777216 // 16 MB payload upper bound
// Wire protocol handshake version.
#define PROTOCOL_VERSION        8
#define WIRE_ID_LEN             13       // length of the hashed client-order-id token in comments

//---- Packet kinds (protocol v5+)
#define PKT_REQUEST       0
#define PKT_RESPONSE      1
#define PKT_EVENT         2

//---- Event kinds (protocol v5+)
#define EVENT_TICK        1
#define EVENT_TRADE       2
#define EVENT_BOOK        3
#define EVENT_BAR         4

//---- Wire transaction types (protocol v6+)
#define WTX_OTHER         0
#define WTX_DEAL_ADD      1
#define WTX_ORDER_ADD     2
#define WTX_ORDER_DELETE  3
#define WTX_ORDER_UPDATE  4
#define WTX_POSITION      5
#define WTX_REQUEST       6

//---- Wire level modification modes
#define WIRE_LEVEL_KEEP   0
#define WIRE_LEVEL_CLEAR  1
#define WIRE_LEVEL_SET    2

//---- Protocol commands
#define CMD_INIT              1
#define CMD_SHUTDOWN          2
#define CMD_RATES             3
#define CMD_ACCOUNT           4
#define CMD_ORDER_SEND        5
#define CMD_ORDER_CLOSE       6
#define CMD_ORDER_MODIFY      7
#define CMD_SYM_TICK          8
#define CMD_SYM_INFO          9
#define CMD_POSITIONS_GET     10
#define CMD_ORDERS_GET        11
#define CMD_DEALS_GET         12
#define CMD_SUBSCRIBE_TICKS   13
#define CMD_UNSUBSCRIBE_TICKS 14
#define CMD_SUBSCRIBE_TRADE   15
#define CMD_UNSUBSCRIBE_TRADE 16
#define CMD_SUBSCRIBE_BOOK    17
#define CMD_UNSUBSCRIBE_BOOK  18
#define CMD_SUBSCRIBE_BARS    19
#define CMD_UNSUBSCRIBE_BARS  20
#define CMD_PING              21
#define CMD_SYMBOLS_TOTAL     22
#define CMD_SYMBOLS_GET       23
#define CMD_SYMBOL_SELECT     24
#define CMD_CALENDAR_EVENTS   25
#define CMD_CALENDAR_COUNTRIES 26
#define CMD_STATS             27

enum ENUM_SERVER_TIME_CONVENTION {
    SERVER_TIME_FIXED = 0,
    SERVER_TIME_NEW_YORK_DST = 1,
    SERVER_TIME_EUROPE_DST = 2
};

//---- EA input
input int    InpMagicNumber          = 20240101;    // Magic number for bridge orders
input string InpPipeName             = "mt5bridge"; // Custom Named Pipe Name
input string InpPipeSecret           = "";          // Shared secret for client authentication (required by default)
input bool   InpRequireSecret        = true;        // Require non-empty InpPipeSecret on initialization (set false to opt out)
input bool   InpEnforceMagicNumber   = true;       // Reject close/modify/send for magic not matching InpMagicNumber
input string InpPipeSDDL             = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"; // SDDL security descriptor restricting access to owner, local system & admins
input bool   InpConvertToUTC         = true;        // Convert history timestamps to UTC (set false for raw broker time)
input ENUM_SERVER_TIME_CONVENTION InpServerTimeConvention = SERVER_TIME_NEW_YORK_DST; // Broker server clock convention when converting to UTC
input int    InpServerUtcOffsetHours = 2;           // Fixed server UTC offset (-14..14), used with SERVER_TIME_FIXED
input int    InpTimerIntervalMs      = 1;           // Timer polling interval in milliseconds (default 1ms on tuned VPS)
input int    InpMaxRequestsPerTimer  = 64;          // Max pipe requests serviced per timer tick (prevents UI starvation)
input uint   InpMaxTimerBudgetUs     = 5000;        // Max execution budget per timer tick in microseconds (5000us = 5ms)
input bool   InpAutoSelectSymbols    = true;        // Automatically select queried/traded symbols into Market Watch
input long   InpConfigGeneration     = 0;           // Monotonic config version; a higher value takes over an older running instance
input int    InpTakeoverTimeoutMs    = 5000;        // Max wait for the existing instance to release the pipe (ms)
input bool   InpCloseStaleCharts     = false;       // Close every other chart after taking over (headless: one chart only)
input bool   InpEnableKillSwitch     = false;       // Emergency kill switch: immediately block all outgoing orders
input bool   InpRequireLiveAck       = true;        // Require explicit acknowledgement when running on a live (real) account
input bool   InpLiveAckAcknowledged  = false;       // Set true to acknowledge live account trading
input double InpMaxVolumePerOrder    = 10.0;        // Max volume allowed for a single order (0 = unlimited)
input uint   InpMaxDeviation         = 100;         // Max allowed slippage deviation in points (0 = unlimited)

// Ensure symbol is available in Market Watch, respecting InpAutoSelectSymbols
bool EnsureSymbolAvailable(string sym) {
    if (SymbolInfoInteger(sym, SYMBOL_SELECT) != 0)
        return true;
    if (InpAutoSelectSymbols)
        return SymbolSelect(sym, true);
    Print("MT5Bridge: symbol '", sym, "' is not in Market Watch and InpAutoSelectSymbols is disabled");
    return false;
}

// Automatically select the broker-supported order filling mode for a symbol
ENUM_ORDER_TYPE_FILLING GetSymbolFillingMode(string sym) {
    uint filling = (uint)SymbolInfoInteger(sym, SYMBOL_FILLING_MODE);
    if ((filling & SYMBOL_FILLING_FOK) != 0)
        return ORDER_FILLING_FOK;
    if ((filling & SYMBOL_FILLING_IOC) != 0)
        return ORDER_FILLING_IOC;
    // Fallback based on trade execution mode when SYMBOL_FILLING_MODE reports 0
    long exec = SymbolInfoInteger(sym, SYMBOL_TRADE_EXEMODE);
    if (exec == SYMBOL_TRADE_EXECUTION_MARKET || exec == SYMBOL_TRADE_EXECUTION_INSTANT) {
        return ORDER_FILLING_IOC;
    }
    return ORDER_FILLING_RETURN;
}

// Rounding direction for SnapToTick. Stops are snapped in the protective
// direction so they are never closer to the market than requested.
enum SnapDirection {
    SNAP_NEAREST = 0,
    SNAP_DOWN    = -1,
    SNAP_UP      = 1,
};

// Snap a price level to the broker's tick grid.
//
// Rounding table (market below/above is implied by the level):
//   buy  stop-loss   -> down   (a lower SL is never closer to the market)
//   sell stop-loss   -> up     (a higher SL is never closer)
//   sell take-profit -> down   (a lower TP is never closer)
//   buy  take-profit -> up     (a higher TP is never closer)
//   entry/limit price -> nearest (unchanged legacy behaviour)
//
// Computed stops (ATR trails, R-multiple ladders, breakout levels) are pure
// floating-point values that frequently do not land exactly on a tick multiple.
// Rejecting them outright can leave positions without a valid SL/TP, so levels
// are snapped before tick-size validation and stops-level checks.
double SnapToTick(const double value, const double tick_size, const int digits,
                  const SnapDirection direction = SNAP_NEAREST) {
    if (value <= 0.0) return value;
    double snapped = value;
    if (tick_size > 0.0) {
        const double steps = value / tick_size;
        if (direction == SNAP_DOWN)
            snapped = MathFloor(steps) * tick_size;
        else if (direction == SNAP_UP)
            snapped = MathCeil(steps) * tick_size;
        else
            snapped = MathRound(steps) * tick_size;
    }
    return NormalizeDouble(snapped, digits);
}

// True for the order types whose stop-loss sits below the market.
bool IsBuySideOrder(const ENUM_ORDER_TYPE type) {
    return type == ORDER_TYPE_BUY || type == ORDER_TYPE_BUY_LIMIT || type == ORDER_TYPE_BUY_STOP;
}

//---- State
long g_server  = INVALID_HANDLE;
long g_client  = INVALID_HANDLE;
long g_takeover_event = 0;   // Win32 event handle; 0 = not created
uint g_stale_cleanup_at = 0; // GetTickCount() deadline for chart cleanup; 0 = not scheduled
bool g_stale_charts_closed = false;
bool g_running = false;

// Initialization state. The pipe bind is retried on the timer thread so that
// OnInit returns immediately instead of blocking for the takeover timeout.
enum ENUM_INIT_STATE { INIT_WAIT_TAKEOVER, INIT_READY };
ENUM_INIT_STATE g_init_state = INIT_WAIT_TAKEOVER;
ulong g_takeover_start = 0;   // GetTickCount() when takeover waiting began

bool g_authenticated = false;
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
    ulong start_us = GetMicrosecondCount();
    while (offset < len) {
        uint chunk_len = len - offset;
        uchar chunk[];
        ArrayResize(chunk, chunk_len);
        ArrayCopy(chunk, data, 0, offset, chunk_len);
        uint written = 0;
        if (WriteFile(h, chunk, chunk_len, written, 0) && written > 0) {
            offset += written;
        } else {
            // Buffer full or client not reading: time out after 1000ms (1,000,000 us) to avoid freezing chart thread
            if (GetMicrosecondCount() - start_us > 1000000) {
                Print("MT5Bridge: PipeWriteExact timed out (1000ms), pipe congested or client unresponsive");
                return false;
            }
            Sleep(1);
        }
    }
    return true;
}

// ── Binary unpack helpers (bounds-checked) ───────────────────────────────────

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

bool SafeUnpackU32(const uchar &b[], int &off, uint total_len, uint &val) {
    if ((uint)off + 4 > total_len || (uint)off + 4 > (uint)ArraySize(b)) return false;
    val = UnpackU32(b, off);
    off += 4;
    return true;
}

bool SafeUnpackI32(const uchar &b[], int &off, uint total_len, int &val) {
    uint u = 0;
    if (!SafeUnpackU32(b, off, total_len, u)) return false;
    val = (int)u;
    return true;
}

bool SafeUnpackI64(const uchar &b[], int &off, uint total_len, long &val) {
    if ((uint)off + 8 > total_len || (uint)off + 8 > (uint)ArraySize(b)) return false;
    val = UnpackI64(b, off);
    off += 8;
    return true;
}

bool SafeUnpackU64(const uchar &b[], int &off, uint total_len, ulong &val) {
    long l = 0;
    if (!SafeUnpackI64(b, off, total_len, l)) return false;
    val = (ulong)l;
    return true;
}

bool SafeUnpackF64(const uchar &b[], int &off, uint total_len, double &val) {
    if ((uint)off + 8 > total_len || (uint)off + 8 > (uint)ArraySize(b)) return false;
    val = UnpackF64(b, off);
    off += 8;
    return true;
}

bool SafeUnpackU8(const uchar &b[], int &off, uint total_len, uchar &val) {
    if ((uint)off + 1 > total_len || (uint)off + 1 > (uint)ArraySize(b)) return false;
    val = b[off];
    off += 1;
    return true;
}

// Unpack length-prefixed UTF-8 string with strict bounds verification.
bool SafeUnpackStr(const uchar &b[], int &off, uint total_len, string &s) {
    uint str_len = 0;
    if (!SafeUnpackU32(b, off, total_len, str_len)) return false;
    if (str_len == 0) {
        s = "";
        return true;
    }
    if ((uint)off + str_len > total_len || (uint)off + str_len > (uint)ArraySize(b)) return false;
    s = CharArrayToString(b, off, (int)str_len, CP_UTF8);
    off += (int)str_len;
    return true;
}

// ── Binary pack helpers ──────────────────────────────────────────────────────

void PackU8(uchar &b[], uchar v) {
    int n = ArraySize(b); ArrayResize(b, n + 1);
    b[n] = v;
}

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

// The number of bytes reserved for the account currency in the account
// response.
#define ACCOUNT_CURRENCY_FIELD 32

// Pack a NUL-padded, fixed-size (32 byte) string field.
void PackFixedString32(uchar &data[], const string s) {
    uchar str_bytes[32];
    ArrayInitialize(str_bytes, 0);
    StringToCharArray(s, str_bytes, 0, 31);
    int cur_len = ArraySize(data);
    ArrayResize(data, cur_len + 32);
    ArrayCopy(data, str_bytes, cur_len, 0, 32);
}

void PackF64(uchar &b[], double v) {
    DoubleBytes db; db.v = v;
    uchar tmp[8];
    StructToCharArray(db, tmp);
    int n = ArraySize(b); ArrayResize(b, n + 8);
    ArrayCopy(b, tmp, n, 0, 8);
}

// Pack a length-prefixed UTF-8 string: [u32 name_len][name bytes].
// MT5 symbol names are ASCII, so StringToCharArray/CharArrayToString with CP_UTF8 agree.
void PackStr(uchar &b[], const string s) {
    uchar bytes[256];
    ArrayInitialize(bytes, 0);
    int len = StringToCharArray(s, bytes, 0, 255);
    PackU32(b, (uint)len);
    int n = ArraySize(b);
    ArrayResize(b, n + len);
    ArrayCopy(b, bytes, n, 0, len);
}

// ── Response senders (protocol v6+) ─────────────────────────────────────────

uint  g_current_cmd    = 0;
uint  g_current_req_id = 0;
ulong g_event_seq      = 0;

bool SendResponse(long h, int status, const uchar &data[], uint data_len) {
    uchar hdr[16];
    // length (4 bytes)
    hdr[0] = (uchar)(data_len & 0xFF);
    hdr[1] = (uchar)((data_len >> 8)  & 0xFF);
    hdr[2] = (uchar)((data_len >> 16) & 0xFF);
    hdr[3] = (uchar)((data_len >> 24) & 0xFF);
    // kind (1 byte) = PKT_RESPONSE (1)
    hdr[4] = (uchar)PKT_RESPONSE;
    // _pad (1 byte) = 0
    hdr[5] = 0;
    // id (2 bytes) = g_current_cmd
    hdr[6] = (uchar)(g_current_cmd & 0xFF);
    hdr[7] = (uchar)((g_current_cmd >> 8) & 0xFF);
    // status (4 bytes)
    hdr[8]  = (uchar)(status & 0xFF);
    hdr[9]  = (uchar)((status >> 8)  & 0xFF);
    hdr[10] = (uchar)((status >> 16) & 0xFF);
    hdr[11] = (uchar)((status >> 24) & 0xFF);
    // request_id (4 bytes) = g_current_req_id
    hdr[12] = (uchar)(g_current_req_id & 0xFF);
    hdr[13] = (uchar)((g_current_req_id >> 8)  & 0xFF);
    hdr[14] = (uchar)((g_current_req_id >> 16) & 0xFF);
    hdr[15] = (uchar)((g_current_req_id >> 24) & 0xFF);

    if (!PipeWriteExact(h, hdr, 16)) return false;
    if (data_len > 0 && !PipeWriteExact(h, data, data_len)) return false;
    FlushFileBuffers(h);
    return true;
}

bool SendEvent(long h, uint event_type, const uchar &data[], uint data_len) {
    uchar hdr[16];
    // length (4 bytes)
    hdr[0] = (uchar)(data_len & 0xFF);
    hdr[1] = (uchar)((data_len >> 8)  & 0xFF);
    hdr[2] = (uchar)((data_len >> 16) & 0xFF);
    hdr[3] = (uchar)((data_len >> 24) & 0xFF);
    // kind (1 byte) = PKT_EVENT (2)
    hdr[4] = (uchar)PKT_EVENT;
    // _pad (1 byte) = 0
    hdr[5] = 0;
    // id (2 bytes) = event_type
    hdr[6] = (uchar)(event_type & 0xFF);
    hdr[7] = (uchar)((event_type >> 8) & 0xFF);
    // status (4 bytes) = 0
    hdr[8]  = 0;
    hdr[9]  = 0;
    hdr[10] = 0;
    hdr[11] = 0;
    // request_id (4 bytes) = 0
    hdr[12] = 0;
    hdr[13] = 0;
    hdr[14] = 0;
    hdr[15] = 0;

    if (!PipeWriteExact(h, hdr, 16)) return false;
    if (data_len > 0 && !PipeWriteExact(h, data, data_len)) return false;
    // Note: FlushFileBuffers removed from per-event path to avoid blocking MT5 event loop.
    // Flushes are performed once per batch in FlushTickQueue/FlushTradeQueue/OnBookEvent.
    return true;
}

bool SendOk(long h) {
    uchar empty[1]; // unused
    return SendResponse(h, 1, empty, 0);
}

bool SendOkData(long h, const uchar &data[], uint len) {
    return SendResponse(h, 1, data, len);
}

bool SendError(long h) {
    uchar empty[1];
    return SendResponse(h, -1, empty, 0);
}

// ── Command handlers ──────────────────────────────────────────────────────────

void HandleInit(long h, const uchar &payload[], uint len) {
    int off = 0;
    long req_login = 0;
    string req_password = "";
    string req_server = "";

    if (!SafeUnpackI64(payload, off, len, req_login) ||
        !SafeUnpackStr(payload, off, len, req_password) ||
        !SafeUnpackStr(payload, off, len, req_server)) {
        g_authenticated = false;
        SendError(h);
        return;
    }

    // Verify wire protocol version (mandatory)
    uint proto_ver = 0;
    if (!SafeUnpackU32(payload, off, len, proto_ver) || proto_ver != PROTOCOL_VERSION) {
        Print("MT5Bridge: auth failed — protocol version mismatch or missing (client=", proto_ver,
              ", server=", PROTOCOL_VERSION, ")");
        g_authenticated = false;
        SendError(h);
        return;
    }

    long actual_login = AccountInfoInteger(ACCOUNT_LOGIN);
    string actual_server = AccountInfoString(ACCOUNT_SERVER);

    // If a shared pipe secret is configured on the EA, verify client supplied exact match
    if (StringLen(InpPipeSecret) > 0 && req_password != InpPipeSecret) {
        Print("MT5Bridge: auth failed — invalid shared secret / pipe token");
        g_authenticated = false;
        SendError(h);
        return;
    }

    // If client supplied a login (> 0), verify it matches the active MT5 terminal account
    if (req_login > 0 && req_login != actual_login) {
        Print("MT5Bridge: auth failed — requested login ", req_login,
              " does not match terminal login ", actual_login);
        g_authenticated = false;
        SendError(h);
        return;
    }

    // If client supplied a server, verify server name matches (case-insensitive exact comparison)
    if (StringLen(req_server) > 0) {
        string s1 = req_server;
        string s2 = actual_server;
        StringToUpper(s1);
        StringToUpper(s2);
        if (s1 != s2) {
            Print("MT5Bridge: auth failed — requested server '", req_server,
                  "' does not match terminal server '", actual_server, "'");
            g_authenticated = false;
            SendError(h);
            return;
        }
    }

    g_authenticated = true;
    SendOk(h);
    Print("MT5Bridge: client authenticated (account: ", actual_login, ", server: ", actual_server, ")");
}

void HandleShutdown(long h) {
    SendOk(h);
    g_authenticated = false;
    Print("MT5Bridge: client requested shutdown");
    // Client disconnect will be handled in the main loop.
}

bool g_server_time_warning_printed = false;

bool IsNewYorkDst(datetime t) {
    MqlDateTime dt;
    TimeToStruct(t, dt);

    bool is_dst = false;
    if (dt.mon > 3 && dt.mon < 11) {
        is_dst = true;
    } else if (dt.mon == 3) {
        int day_of_week_of_1st = (dt.day_of_week - (dt.day - 1) % 7 + 7) % 7;
        int first_sun = (day_of_week_of_1st == 0) ? 1 : (8 - day_of_week_of_1st);
        int second_sun = first_sun + 7;
        if (dt.day > second_sun || (dt.day == second_sun && dt.hour >= 2)) {
            is_dst = true;
        }
    } else if (dt.mon == 11) {
        int day_of_week_of_1st = (dt.day_of_week - (dt.day - 1) % 7 + 7) % 7;
        int first_sun = (day_of_week_of_1st == 0) ? 1 : (8 - day_of_week_of_1st);
        if (dt.day < first_sun || (dt.day == first_sun && dt.hour < 2)) {
            is_dst = true;
        }
    }
    return is_dst;
}

int LastSunday(int year, int month) {
    // First day of the next month; the last day of the current month is one
    // day (86400 seconds) before it. This handles every month length and
    // leap years without a days-in-month table.
    int next_month = month + 1;
    int next_year = year;
    if (next_month > 12) {
        next_month = 1;
        next_year = year + 1;
    }

    MqlDateTime dt;
    dt.year = next_year;
    dt.mon = next_month;
    dt.day = 1;
    dt.hour = 0;
    dt.min = 0;
    dt.sec = 0;
    datetime first_of_next = StructToTime(dt);
    datetime last_of_current = first_of_next - 86400;

    MqlDateTime last_dt;
    TimeToStruct(last_of_current, last_dt);

    // Go back to the previous Sunday (day_of_week: 0=Sunday, ..., 6=Saturday).
    return last_dt.day - last_dt.day_of_week;
}

bool IsEuropeDst(datetime t) {
    MqlDateTime dt;
    TimeToStruct(t, dt);
    if (dt.mon > 3 && dt.mon < 10) {
        return true;
    }
    if (dt.mon < 3 || dt.mon > 10) {
        return false;
    }

    int last_sunday = LastSunday(dt.year, dt.mon);
    if (dt.mon == 3) {
        return dt.day > last_sunday || (dt.day == last_sunday && dt.hour >= 2);
    }
    return dt.day < last_sunday || (dt.day == last_sunday && dt.hour < 3);
}

bool TryObservedServerUtcOffset(long &offset) {
    offset = (long)TimeTradeServer() - (long)TimeGMT();
    if (offset < -14 * 3600 || offset > 14 * 3600) {
        return false;
    }
    long remainder = offset % 60;
    if (remainder >= 30) offset += 60 - remainder;
    else if (remainder <= -30) offset -= 60 + remainder;
    else offset -= remainder;
    return true;
}

long ExpectedServerUtcOffset(datetime t) {
    if (InpServerTimeConvention == SERVER_TIME_FIXED) {
        return (long)InpServerUtcOffsetHours * 3600;
    }
    if (InpServerTimeConvention == SERVER_TIME_EUROPE_DST) {
        return (IsEuropeDst(t) ? 3 : 2) * 3600;
    }
    return (IsNewYorkDst(t) ? 3 : 2) * 3600;
}

string ServerTimeConventionName() {
    if (InpServerTimeConvention == SERVER_TIME_FIXED) return "fixed";
    if (InpServerTimeConvention == SERVER_TIME_EUROPE_DST) return "europe_dst";
    return "new_york_dst";
}

// Computes server-to-UTC offset for a given timestamp and checks the configured
// convention against the terminal's current observed server offset.
long GetServerUtcOffset(datetime t) {
    if (!InpConvertToUTC) return 0;

    long observed = 0;
    if (!TryObservedServerUtcOffset(observed)) {
        if (!g_server_time_warning_printed) {
            Print("MT5Bridge: unable to observe a valid server-to-UTC offset; UTC conversion is disabled for this value");
            g_server_time_warning_printed = true;
        }
        return 0;
    }

    long expected_now = ExpectedServerUtcOffset(TimeTradeServer());
    if (observed != expected_now) {
        if (!g_server_time_warning_printed) {
            Print("MT5Bridge: configured server-time convention disagrees with observed offset (expected ",
                  expected_now, "s, observed ", observed,
                  "s); using the observed offset until the EA is configured correctly");
            g_server_time_warning_printed = true;
        }
        return observed;
    }

    return ExpectedServerUtcOffset(t);
}

datetime ConvertServerTimeToUTC(datetime t) {
    long sec_diff = GetServerUtcOffset(t);
    return (datetime)((long)t - sec_diff);
}

// Current server time in epoch milliseconds, normalized to UTC when InpConvertToUTC is enabled.
long ServerTimeMsc() {
    long sec = (long)TimeTradeServer();
    if (InpConvertToUTC) {
        long diff = (long)TimeTradeServer() - (long)TimeGMT();
        if (diff >= -14 * 3600 && diff <= 14 * 3600) {
            long rem = diff % 60;
            if (rem >= 30) diff += (60 - rem);
            else if (rem <= -30) diff -= (60 + rem);
            else diff -= rem;
            sec -= diff;
        }
    }
    return sec * 1000;
}

void HandleCopyRates(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    int tf = 0;
    long from = 0;
    long to = 0;

    if (!SafeUnpackStr(payload, off, len, sym) ||
        !SafeUnpackI32(payload, off, len, tf) ||
        !SafeUnpackI64(payload, off, len, from) ||
        !SafeUnpackI64(payload, off, len, to)) {
        SendError(h);
        return;
    }

    // Optional max buffer capacity requested by client (avoids over-allocating MQL5 payload)
    int max_bars = 0;
    SafeUnpackI32(payload, off, len, max_bars);

    if (!EnsureSymbolAvailable(sym)) {
        uchar e[1]; SendResponse(h, 0, e, 0); return;
    }

    ENUM_TIMEFRAMES mtf     = (ENUM_TIMEFRAMES)tf;
    datetime        dt_from = (datetime)(from + GetServerUtcOffset((datetime)from));
    datetime        dt_to   = (datetime)(to + GetServerUtcOffset((datetime)to));

    MqlRates rates[];
    ArraySetAsSeries(rates, false);

    // Try reading directly from MT5 local cache first to avoid hammering broker history server
    ResetLastError();
    int filled = CopyRates(sym, mtf, dt_from, dt_to, rates);

    // If local cache does not have the required range yet, request terminal to sync from server
    if (filled <= 0) {
        datetime now = TimeTradeServer();
        if (dt_from < now) {
            int required_bars = (int)((now - dt_from) / PeriodSeconds(mtf)) + 100;
            if (required_bars > 0) {
                MqlRates dummy[];
                CopyRates(sym, mtf, 0, required_bars, dummy);
            }
        }
        ResetLastError();
        filled = CopyRates(sym, mtf, dt_from, dt_to, rates);
    }

    if (filled <= 0) { uchar e[1]; SendResponse(h, 0, e, 0); return; }

    int send_bars = filled;
    if (max_bars > 0 && send_bars > max_bars) {
        send_bars = max_bars;
    }

    // Serialise MqlRates[] into Mt5Rate bytes (layouts match — 60 bytes each).
    uchar data[];
    ArrayResize(data, send_bars * 60);
    for (int i = 0; i < send_bars; i++) {
        if (InpConvertToUTC) {
            rates[i].time = ConvertServerTimeToUTC(rates[i].time);
        }
        uchar tmp[];
        StructToCharArray(rates[i], tmp);
        ArrayCopy(data, tmp, i * 60, 0, 60);
    }
    SendResponse(h, send_bars, data, (uint)(send_bars * 60));
}

void HandleAccount(long h) {
    double balance    = AccountInfoDouble(ACCOUNT_BALANCE);
    double equity     = AccountInfoDouble(ACCOUNT_EQUITY);
    double margin     = AccountInfoDouble(ACCOUNT_MARGIN);
    double free_margin= AccountInfoDouble(ACCOUNT_MARGIN_FREE);
    // The account leverage: the bridge needs it to estimate the margin a
    // position requires, and a configured value can be wrong (or stale).
    double leverage   = (double)AccountInfoInteger(ACCOUNT_LEVERAGE);
    // The deposit currency: the bridge books the P&L in it, and assuming
    // "USD" is wrong for every other account.
    string   currency = AccountInfoString(ACCOUNT_CURRENCY);

    uchar data[];
    PackF64(data, balance);
    PackF64(data, equity);
    PackF64(data, margin);
    PackF64(data, free_margin);
    PackF64(data, leverage);
    PackFixedString32(data, currency);
    SendOkData(h, data, (uint)ArraySize(data));
}

// Intermediate struct for trade results (wire format packed via Pack* helpers).
struct BridgeTradeResult {
    uint   retcode;
    ulong  deal;
    ulong  order;
    ulong  position;
    double volume;
    double price;
};

//---- Order idempotency cache (stores recent executions to prevent duplicate fills on retry)
struct OrderExecutionRecord {
    string client_order_id;   // 13-char wire token (see ExtractClientOrderId), NOT the raw client ID
    string symbol;            // request fingerprint: a replay is only valid for the SAME request
    int    otype;
    double req_volume;
    uint   retcode;
    ulong  deal;
    ulong  order;
    ulong  position;
    double volume;
    double price;
    datetime timestamp;
};

#define ORDER_CACHE_CAPACITY 512
OrderExecutionRecord g_order_cache[ORDER_CACHE_CAPACITY];
int g_order_cache_count = 0;
int g_order_cache_head  = 0;

void CacheOrderExecution(const string cid, const string req_symbol, int req_otype, double req_volume,
                         uint retcode, ulong deal, ulong order, ulong pos, double vol, double price) {
    if (StringLen(cid) == 0) return;
    g_order_cache[g_order_cache_head].client_order_id = cid;
    g_order_cache[g_order_cache_head].symbol          = req_symbol;
    g_order_cache[g_order_cache_head].otype           = req_otype;
    g_order_cache[g_order_cache_head].req_volume      = req_volume;
    g_order_cache[g_order_cache_head].retcode         = retcode;
    g_order_cache[g_order_cache_head].deal            = deal;
    g_order_cache[g_order_cache_head].order           = order;
    g_order_cache[g_order_cache_head].position        = pos;
    g_order_cache[g_order_cache_head].volume          = vol;
    g_order_cache[g_order_cache_head].price           = price;
    g_order_cache[g_order_cache_head].timestamp       = TimeCurrent();
    g_order_cache_head = (g_order_cache_head + 1) % ORDER_CACHE_CAPACITY;
    if (g_order_cache_count < ORDER_CACHE_CAPACITY) g_order_cache_count++;
}

// Returns the ring-buffer index of a live (within 24 h) cached execution for this token, or -1.
int FindCachedOrderIndex(const string cid) {
    if (StringLen(cid) == 0) return -1;
    datetime cutoff = TimeCurrent() - 86400; // 24-hour cache TTL
    for (int i = 0; i < g_order_cache_count; i++) {
        if (g_order_cache[i].client_order_id == cid && g_order_cache[i].timestamp >= cutoff) {
            return i;
        }
    }
    return -1;
}

// Recover an execution from terminal history when the in-memory cache was lost
// (for example, the terminal restarted between the send and the client's retry).
// The cid token is part of the order comment, which persists in account history,
// so it still identifies the original execution after a restart.
bool FindHistoricalExecution(const string cid, const string req_symbol, int req_otype,
                             double req_volume, uint &retcode, ulong &deal, ulong &order,
                             ulong &position, double &vol, double &price) {
    if (StringLen(cid) == 0 || StringLen(req_symbol) == 0) return false;

    datetime to = TimeCurrent() + 60;
    datetime from = to - 86400 - 3600;
    if (!HistorySelect(from, to)) return false;

    int total = HistoryOrdersTotal();
    for (int i = total - 1; i >= 0; i--) {
        ulong ticket = HistoryOrderGetTicket(i);
        if (ticket == 0) continue;

        string cmt = HistoryOrderGetString(ticket, ORDER_COMMENT);
        if (ExtractClientOrderId(cmt) != cid) continue;
        if (HistoryOrderGetString(ticket, ORDER_SYMBOL) != req_symbol) continue;
        if ((int)HistoryOrderGetInteger(ticket, ORDER_TYPE) != req_otype) continue;

        double requested = HistoryOrderGetDouble(ticket, ORDER_VOLUME_INITIAL);
        if (MathAbs(requested - req_volume) > 1e-9) continue;

        int state = (int)HistoryOrderGetInteger(ticket, ORDER_STATE);
        if (state == ORDER_STATE_PLACED) {
            // The original pending order is still live. Returning its ticket is
            // safer than placing a second order with the same idempotency key.
            retcode = TRADE_RETCODE_PLACED;
            deal = 0;
            order = ticket;
            position = ticket;
            vol = 0.0;
            price = 0.0;
            return true;
        }
        if (state != ORDER_STATE_FILLED && state != ORDER_STATE_PARTIAL) continue;

        double filled = 0.0;
        double notional = 0.0;
        ulong last_deal = 0;
        ulong pos_id = 0;
        int deals = HistoryDealsTotal();
        for (int j = 0; j < deals; j++) {
            ulong dticket = HistoryDealGetTicket(j);
            if (dticket == 0) continue;
            if ((ulong)HistoryDealGetInteger(dticket, DEAL_ORDER) != ticket) continue;

            double dv = HistoryDealGetDouble(dticket, DEAL_VOLUME);
            if (dv <= 0.0) continue;
            filled += dv;
            notional += dv * HistoryDealGetDouble(dticket, DEAL_PRICE);
            last_deal = dticket;

            ulong pid = (ulong)HistoryDealGetInteger(dticket, DEAL_POSITION_ID);
            if (pid > 0) pos_id = pid;
        }
        if (filled <= 0.0 || last_deal == 0) continue;

        retcode = TRADE_RETCODE_DONE;
        deal = last_deal;
        order = ticket;
        position = (pos_id > 0) ? pos_id : ticket;
        vol = filled;
        price = notional / filled;
        return true;
    }
    return false;
}

// Crockford base32 alphabet used by the Rust client's wire_id(): 0-9 and A-Z without I, L, O, U.
bool IsWireTokenChar(const ushort c) {
    if (c >= '0' && c <= '9') return true;
    if (c < 'A' || c > 'Z') return false;
    return (c != 'I' && c != 'L' && c != 'O' && c != 'U');
}

// Extract the idempotency key from an order comment.
//
// Returns the 13-character wire token ONLY for a well-formed identity prefix
//     cid:<13 Crockford-base32 chars>            or
//     cid:<13 Crockford-base32 chars>:<free text>
// and "" for everything else. In particular a plain free-text comment (e.g. "scalp") is NEVER an
// idempotency key: earlier versions returned the whole comment, so two unrelated orders sharing a
// comment were treated as duplicates and the second silently received the first one's execution.
// The token is a hash of the FULL client_order_id computed by the client, so distinct IDs that
// merely share a long prefix can no longer collide (the old scheme truncated the raw ID).
string ExtractClientOrderId(const string cmt) {
    if (StringFind(cmt, "cid:") != 0) return "";
    int n = StringLen(cmt);
    if (n < 4 + WIRE_ID_LEN) return "";
    for (int i = 4; i < 4 + WIRE_ID_LEN; i++) {
        if (!IsWireTokenChar(StringGetCharacter(cmt, i))) return "";
    }
    if (n > 4 + WIRE_ID_LEN && StringGetCharacter(cmt, 4 + WIRE_ID_LEN) != ':') return "";
    return StringSubstr(cmt, 4, WIRE_ID_LEN);
}

void HandleOrderSend(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    int otype = 0;
    double volume = 0.0, price = 0.0, sl = 0.0, tp = 0.0;
    string comment = "";

    if (!SafeUnpackStr(payload, off, len, sym) ||
        !SafeUnpackI32(payload, off, len, otype) ||
        !SafeUnpackF64(payload, off, len, volume) ||
        !SafeUnpackF64(payload, off, len, price) ||
        !SafeUnpackF64(payload, off, len, sl) ||
        !SafeUnpackF64(payload, off, len, tp) ||
        !SafeUnpackStr(payload, off, len, comment)) {
        SendError(h);
        return;
    }

    // Idempotency check: if client_order_id was passed, check cache to prevent duplicate fills
    string cid = "";
    if (StringLen(comment) > 0) {
        cid = ExtractClientOrderId(comment);
    }

    int cache_idx = (StringLen(cid) > 0) ? FindCachedOrderIndex(cid) : -1;
    if (cache_idx < 0 && StringLen(cid) > 0) {
        uint hist_retcode = 0;
        ulong hist_deal = 0, hist_order = 0, hist_position = 0;
        double hist_volume = 0.0, hist_price = 0.0;
        if (FindHistoricalExecution(cid, sym, otype, volume,
                                    hist_retcode, hist_deal, hist_order, hist_position,
                                    hist_volume, hist_price)) {
            Print("MT5Bridge: recovered idempotent execution for key=", cid,
                  " from terminal history (deal=", hist_deal, ", order=", hist_order, ")");
            CacheOrderExecution(cid, sym, otype, volume, hist_retcode, hist_deal, hist_order,
                                hist_position, hist_volume, hist_price);
            cache_idx = FindCachedOrderIndex(cid);
        }
    }
    if (cache_idx >= 0) {
        // Defence in depth: only replay an execution recorded for the SAME request. If the key matches
        // but symbol/type/volume differ, replaying would hand this caller someone else's fill, so refuse.
        if (g_order_cache[cache_idx].symbol != sym ||
            g_order_cache[cache_idx].otype != otype ||
            MathAbs(g_order_cache[cache_idx].req_volume - volume) > 1e-9) {
            Print("MT5Bridge: REJECTED order — idempotency key ", cid, " was already used for a different request (",
                  g_order_cache[cache_idx].symbol, " type=", g_order_cache[cache_idx].otype,
                  " vol=", g_order_cache[cache_idx].req_volume, "); refusing to replay it for ",
                  sym, " type=", otype, " vol=", volume);
            SendError(h);
            return;
        }
        Print("MT5Bridge: idempotent order replay for key=", cid, " (returning cached deal=", g_order_cache[cache_idx].deal,
              ", order=", g_order_cache[cache_idx].order, ")");
        uchar cdata[];
        PackU32(cdata, g_order_cache[cache_idx].retcode);
        PackU64(cdata, g_order_cache[cache_idx].deal);
        PackU64(cdata, g_order_cache[cache_idx].order);
        PackU64(cdata, g_order_cache[cache_idx].position);
        PackF64(cdata, g_order_cache[cache_idx].volume);
        PackF64(cdata, g_order_cache[cache_idx].price);
        SendOkData(h, cdata, (uint)ArraySize(cdata));
        return;
    }

    uint deviation = 10;
    SafeUnpackU32(payload, off, len, deviation);
    if (deviation == 0) deviation = 10;
    if (InpMaxDeviation > 0 && deviation > InpMaxDeviation) {
        deviation = InpMaxDeviation;
    }

    long expiration = 0;
    SafeUnpackI64(payload, off, len, expiration);

    ulong magic = 0;
    SafeUnpackU64(payload, off, len, magic);

    // If magic enforcement is enabled, reject any order whose custom magic does not match InpMagicNumber
    if (InpEnforceMagicNumber && InpMagicNumber > 0 && magic > 0 && (long)magic != InpMagicNumber) {
        Print("MT5Bridge: rejected order — order magic (", magic,
              ") does not match EA magic (", InpMagicNumber, ")");
        SendError(h);
        return;
    }

    // Emergency kill switch check
    if (InpEnableKillSwitch) {
        Print("MT5Bridge: order rejected — emergency kill switch is active");
        SendError(h);
        return;
    }

    // Max volume limit check
    if (InpMaxVolumePerOrder > 0.0 && volume > InpMaxVolumePerOrder) {
        Print("MT5Bridge: order rejected — volume ", DoubleToString(volume, 2),
              " exceeds InpMaxVolumePerOrder (", DoubleToString(InpMaxVolumePerOrder, 2), ")");
        SendError(h);
        return;
    }

    // EA-side pre-flight validation
    if (!EnsureSymbolAvailable(sym)) {
        Print("MT5Bridge: symbol '", sym, "' not available in Market Watch");
        SendError(h);
        return;
    }

    long trade_mode = SymbolInfoInteger(sym, SYMBOL_TRADE_MODE);
    if (trade_mode == SYMBOL_TRADE_MODE_DISABLED) {
        Print("MT5Bridge: trading is disabled for symbol ", sym);
        SendError(h);
        return;
    }

    if (!MathIsValidNumber(volume) || volume <= 0.0) {
        Print("MT5Bridge: invalid order volume: ", volume);
        SendError(h);
        return;
    }

    double min_vol = SymbolInfoDouble(sym, SYMBOL_VOLUME_MIN);
    double max_vol = SymbolInfoDouble(sym, SYMBOL_VOLUME_MAX);
    if (min_vol > 0.0 && volume < min_vol) {
        Print("MT5Bridge: volume ", volume, " below min lot ", min_vol, " for ", sym);
        SendError(h);
        return;
    }
    if (max_vol > 0.0 && volume > max_vol) {
        Print("MT5Bridge: volume ", volume, " exceeds max lot ", max_vol, " for ", sym);
        SendError(h);
        return;
    }

    double step_vol = SymbolInfoDouble(sym, SYMBOL_VOLUME_STEP);
    if (step_vol > 0.0) {
        double steps_zero = volume / step_vol;
        double steps_min  = (min_vol > 0.0) ? (volume - min_vol) / step_vol : steps_zero;
        bool ok_zero = MathAbs(steps_zero - MathRound(steps_zero)) <= 1e-4;
        bool ok_min  = MathAbs(steps_min  - MathRound(steps_min))  <= 1e-4;
        if (!ok_zero && !ok_min) {
            Print("MT5Bridge: volume ", DoubleToString(volume, 4),
                  " does not align with lot step ", DoubleToString(step_vol, 4),
                  " (min=", DoubleToString(min_vol, 4), ") for ", sym);
            SendError(h);
            return;
        }
    }

    if (!MathIsValidNumber(price) || price < 0.0 ||
        !MathIsValidNumber(sl) || sl < 0.0 ||
        !MathIsValidNumber(tp) || tp < 0.0) {
        Print("MT5Bridge: non-finite or negative price/SL/TP parameters");
        SendError(h);
        return;
    }

    // A stop loss is mandatory. sl == 0.0 is the "no stop" sentinel that MT5
    // understands, so accepting it would place the order unprotected. The
    // Rust side refuses such orders too; the EA enforces it as well so a
    // malformed request from any other client cannot slip through.
    if (sl <= 0.0) {
        Print("MT5Bridge: refusing an order without a stop loss (sl=", DoubleToString(sl, 8), ")");
        SendError(h);
        return;
    }

    ENUM_TRADE_REQUEST_ACTIONS action = TRADE_ACTION_DEAL;
    ENUM_ORDER_TYPE order_type = ORDER_TYPE_BUY;
    double req_price = price;

    switch (otype) {
        case 0: // Market Buy
            action     = TRADE_ACTION_DEAL;
            order_type = ORDER_TYPE_BUY;
            req_price  = SymbolInfoDouble(sym, SYMBOL_ASK);
            break;
        case 1: // Market Sell
            action     = TRADE_ACTION_DEAL;
            order_type = ORDER_TYPE_SELL;
            req_price  = SymbolInfoDouble(sym, SYMBOL_BID);
            break;
        case 2: // Buy Limit
            action     = TRADE_ACTION_PENDING;
            order_type = ORDER_TYPE_BUY_LIMIT;
            break;
        case 3: // Sell Limit
            action     = TRADE_ACTION_PENDING;
            order_type = ORDER_TYPE_SELL_LIMIT;
            break;
        case 4: // Buy Stop
            action     = TRADE_ACTION_PENDING;
            order_type = ORDER_TYPE_BUY_STOP;
            break;
        case 5: // Sell Stop
            action     = TRADE_ACTION_PENDING;
            order_type = ORDER_TYPE_SELL_STOP;
            break;
        default:
            Print("MT5Bridge: unsupported order type: ", otype);
            SendError(h);
            return;
    }

    // Broker constraint validation: tick size & stops level distance
    double ask = SymbolInfoDouble(sym, SYMBOL_ASK);
    double bid = SymbolInfoDouble(sym, SYMBOL_BID);
    double point = SymbolInfoDouble(sym, SYMBOL_POINT);
    double tick_size = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE);
    if (tick_size <= 0.0) tick_size = point;
    long stops_lvl = SymbolInfoInteger(sym, SYMBOL_TRADE_STOPS_LEVEL);
    double min_stop_dist = (stops_lvl > 0 && point > 0.0) ? (stops_lvl * point) : 0.0;
    int sym_digits = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);

    // Snap computed SL/TP/pending price to the broker tick grid instead of rejecting.
    // The Rust side derives stops with pure float math, so on instruments whose tick size
    // is not 10^-digits (e.g. half-pip ticks on 5-digit symbols) unaligned values would be
    // rejected and could leave positions without a stop; snapping drifts at most half a tick.
    double snapped_p = SnapToTick(req_price, tick_size, sym_digits, SNAP_NEAREST);
    if (req_price > 0.0 && MathAbs(snapped_p - req_price) > 1e-9) {
        Print("MT5Bridge: order price ", DoubleToString(req_price, sym_digits),
              " snapped to ", DoubleToString(snapped_p, sym_digits),
              " (tick size ", DoubleToString(tick_size, sym_digits), ")");
        req_price = snapped_p;
    }
    const bool buy_side = IsBuySideOrder(order_type);
    double snapped_sl = SnapToTick(sl, tick_size, sym_digits, buy_side ? SNAP_DOWN : SNAP_UP);
    if (sl > 0.0 && MathAbs(snapped_sl - sl) > 1e-9) {
        Print("MT5Bridge: SL ", DoubleToString(sl, sym_digits),
              " snapped to ", DoubleToString(snapped_sl, sym_digits),
              " (tick size ", DoubleToString(tick_size, sym_digits), ")");
        sl = snapped_sl;
    }
    double snapped_tp = SnapToTick(tp, tick_size, sym_digits, buy_side ? SNAP_UP : SNAP_DOWN);
    if (tp > 0.0 && MathAbs(snapped_tp - tp) > 1e-9) {
        Print("MT5Bridge: TP ", DoubleToString(tp, sym_digits),
              " snapped to ", DoubleToString(snapped_tp, sym_digits),
              " (tick size ", DoubleToString(tick_size, sym_digits), ")");
        tp = snapped_tp;
    }

    // Check tick size alignment
    if (tick_size > 0.0) {
        if (req_price > 0.0 && action == TRADE_ACTION_PENDING) {
            double p_steps = req_price / tick_size;
            if (MathAbs(p_steps - MathRound(p_steps)) > 1e-4) {
                Print("MT5Bridge: order price ", DoubleToString(req_price, 5),
                      " not aligned to tick size ", DoubleToString(tick_size, 5));
                SendError(h);
                return;
            }
        }
        if (sl > 0.0) {
            double sl_steps = sl / tick_size;
            if (MathAbs(sl_steps - MathRound(sl_steps)) > 1e-4) {
                Print("MT5Bridge: SL ", DoubleToString(sl, 5),
                      " not aligned to tick size ", DoubleToString(tick_size, 5));
                SendError(h);
                return;
            }
        }
        if (tp > 0.0) {
            double tp_steps = tp / tick_size;
            if (MathAbs(tp_steps - MathRound(tp_steps)) > 1e-4) {
                Print("MT5Bridge: TP ", DoubleToString(tp, 5),
                      " not aligned to tick size ", DoubleToString(tick_size, 5));
                SendError(h);
                return;
            }
        }
    }

    // Directional and stops level validation
    if (action == TRADE_ACTION_DEAL) {
        if (order_type == ORDER_TYPE_BUY) {
            if (sl > 0.0 && (bid - sl) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Buy SL ", DoubleToString(sl, 5),
                      " violates stops level (bid=", DoubleToString(bid, 5),
                      ", min_dist=", DoubleToString(min_stop_dist, 5), ")");
                SendError(h);
                return;
            }
            if (tp > 0.0 && (tp - bid) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Buy TP ", DoubleToString(tp, 5),
                      " violates stops level (bid=", DoubleToString(bid, 5),
                      ", min_dist=", DoubleToString(min_stop_dist, 5), ")");
                SendError(h);
                return;
            }
        } else if (order_type == ORDER_TYPE_SELL) {
            if (sl > 0.0 && (sl - ask) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Sell SL ", DoubleToString(sl, 5),
                      " violates stops level (ask=", DoubleToString(ask, 5),
                      ", min_dist=", DoubleToString(min_stop_dist, 5), ")");
                SendError(h);
                return;
            }
            if (tp > 0.0 && (ask - tp) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Sell TP ", DoubleToString(tp, 5),
                      " violates stops level (ask=", DoubleToString(ask, 5),
                      ", min_dist=", DoubleToString(min_stop_dist, 5), ")");
                SendError(h);
                return;
            }
        }
    } else if (action == TRADE_ACTION_PENDING) {
        if (order_type == ORDER_TYPE_BUY_LIMIT && (ask - req_price) < min_stop_dist - 1e-8) {
            Print("MT5Bridge: Buy Limit price ", DoubleToString(req_price, 5),
                  " violates stops level from current ask ", DoubleToString(ask, 5));
            SendError(h);
            return;
        }
        if (order_type == ORDER_TYPE_BUY_STOP && (req_price - ask) < min_stop_dist - 1e-8) {
            Print("MT5Bridge: Buy Stop price ", DoubleToString(req_price, 5),
                  " violates stops level from current ask ", DoubleToString(ask, 5));
            SendError(h);
            return;
        }
        if (order_type == ORDER_TYPE_SELL_LIMIT && (req_price - bid) < min_stop_dist - 1e-8) {
            Print("MT5Bridge: Sell Limit price ", DoubleToString(req_price, 5),
                  " violates stops level from current bid ", DoubleToString(bid, 5));
            SendError(h);
            return;
        }
        if (order_type == ORDER_TYPE_SELL_STOP && (bid - req_price) < min_stop_dist - 1e-8) {
            Print("MT5Bridge: Sell Stop price ", DoubleToString(req_price, 5),
                  " violates stops level from current bid ", DoubleToString(bid, 5));
            SendError(h);
            return;
        }

        if (order_type == ORDER_TYPE_BUY_LIMIT || order_type == ORDER_TYPE_BUY_STOP) {
            if (sl > 0.0 && (req_price - sl) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Buy pending SL ", DoubleToString(sl, 5),
                      " violates stops level relative to price ", DoubleToString(req_price, 5));
                SendError(h);
                return;
            }
            if (tp > 0.0 && (tp - req_price) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Buy pending TP ", DoubleToString(tp, 5),
                      " violates stops level relative to price ", DoubleToString(req_price, 5));
                SendError(h);
                return;
            }
        } else if (order_type == ORDER_TYPE_SELL_LIMIT || order_type == ORDER_TYPE_SELL_STOP) {
            if (sl > 0.0 && (sl - req_price) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Sell pending SL ", DoubleToString(sl, 5),
                      " violates stops level relative to price ", DoubleToString(req_price, 5));
                SendError(h);
                return;
            }
            if (tp > 0.0 && (req_price - tp) < min_stop_dist - 1e-8) {
                Print("MT5Bridge: Sell pending TP ", DoubleToString(tp, 5),
                      " violates stops level relative to price ", DoubleToString(req_price, 5));
                SendError(h);
                return;
            }
        }
    }

    double norm_price = NormalizeDouble(req_price, sym_digits);
    double norm_sl    = (sl > 0.0) ? NormalizeDouble(sl, sym_digits) : 0.0;
    double norm_tp    = (tp > 0.0) ? NormalizeDouble(tp, sym_digits) : 0.0;

    double norm_vol = volume;
    if (step_vol > 0.0) {
        double s = step_vol;
        int d = 0;
        while (d < 8) {
            if (MathAbs(s - MathRound(s)) < 1e-4) break;
            s *= 10.0;
            d++;
        }
        norm_vol = NormalizeDouble(volume, d);
    }

    MqlTradeRequest req = {};
    req.action      = action;
    req.symbol      = sym;
    req.volume      = norm_vol;
    req.type        = order_type;
    req.price       = norm_price;
    req.sl          = norm_sl;
    req.tp          = norm_tp;
    req.deviation   = deviation;
    req.magic       = (magic > 0) ? (long)magic : InpMagicNumber;
    req.comment     = comment;
    req.type_filling= GetSymbolFillingMode(sym);

    if (expiration > 0) {
        req.type_time   = ORDER_TIME_SPECIFIED;
        req.expiration  = (datetime)expiration;
    }

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
        // A requested market quote is not evidence of an execution. Leave the
        // price unset so the Rust side can reconcile the broker deal or fail
        // closed instead of booking a fabricated fill.
    }

    ulong pos_ticket = 0;
    if (res.deal > 0 && HistoryDealSelect(res.deal)) {
        pos_ticket = HistoryDealGetInteger(res.deal, DEAL_POSITION_ID);
    }
    if (pos_ticket == 0 && res.order > 0 && PositionSelectByTicket(res.order)) {
        pos_ticket = res.order;
    }
    if (pos_ticket == 0 && res.order > 0) {
        pos_ticket = res.order;
    }
    // Deliberately do NOT fall back to PositionSelect(sym): in hedging accounts, selecting
    // by symbol returns an arbitrary position, which can cause operations on the wrong ticket.
    // Returning 0 (pending/unknown) is strictly safer than returning the wrong ticket.

    // Serialise as packed wire format: u32 retcode + u64 deal + u64 order +
    // u64 position + f64 volume + f64 price = 44 bytes (matches Mt5TradeResult in mt5_bridge.h).
    uchar data[];
    PackU32(data, res.retcode);
    PackU64(data, res.deal);
    PackU64(data, res.order);
    PackU64(data, pos_ticket);
    PackF64(data, res.volume);
    PackF64(data, exec_price);

    // Accept DONE, PLACED, and DONE_PARTIAL (10010) as successful execution
    if (ok && (res.retcode == TRADE_RETCODE_DONE ||
               res.retcode == TRADE_RETCODE_PLACED ||
               res.retcode == TRADE_RETCODE_DONE_PARTIAL)) {
        if (StringLen(cid) > 0) {
            CacheOrderExecution(cid, sym, otype, volume, res.retcode, res.deal, res.order, pos_ticket, res.volume, exec_price);
        }
        SendOkData(h, data, (uint)ArraySize(data));
    } else {
        Print("MT5Bridge: OrderSend failed retcode=", res.retcode);
        SendResponse(h, -1, data, (uint)ArraySize(data));
    }
}

void HandleOrderClose(long h, const uchar &payload[], uint len) {
    if (len < 8) { SendError(h); return; }
    int off = 0;
    ulong ticket = 0;
    if (!SafeUnpackU64(payload, off, len, ticket)) { SendError(h); return; }

    ulong req_magic = 0;
    if ((uint)off + 8 <= len) {
        SafeUnpackU64(payload, off, len, req_magic);
    }

    uint req_deviation = 10;
    if ((uint)off + 4 <= len) {
        SafeUnpackU32(payload, off, len, req_deviation);
    }

    double req_volume = 0.0;
    if ((uint)off + 8 <= len) {
        SafeUnpackF64(payload, off, len, req_volume);
    }

    // Find position by ticket, or pending order to remove
    if (!PositionSelectByTicket(ticket)) {
        if (OrderSelect(ticket)) {
            long ord_magic = OrderGetInteger(ORDER_MAGIC);
            if (req_magic > 0 && ord_magic != (long)req_magic) {
                Print("MT5Bridge: rejected remove — pending order ticket ", ticket, " magic (", ord_magic,
                      ") does not match requested magic (", req_magic, ")");
                SendError(h);
                return;
            } else if (InpEnforceMagicNumber && InpMagicNumber > 0 && ord_magic != InpMagicNumber) {
                Print("MT5Bridge: rejected remove — pending order ticket ", ticket, " magic (", ord_magic,
                      ") does not match EA magic (", InpMagicNumber, ")");
                SendError(h);
                return;
            } else if (InpMagicNumber > 0 && ord_magic != InpMagicNumber) {
                Print("MT5Bridge: warning — removing pending order ticket ", ticket, " magic (", ord_magic,
                      ") does not match EA magic (", InpMagicNumber, ")");
            }

            MqlTradeRequest pend_req = {};
            pend_req.action = TRADE_ACTION_REMOVE;
            pend_req.order  = ticket;
            pend_req.magic  = (ord_magic > 0) ? ord_magic : InpMagicNumber;

            MqlTradeResult pend_res = {};
            bool pend_ok = OrderSend(pend_req, pend_res);

            uchar pend_data[];
            PackU32(pend_data, pend_res.retcode);
            PackU64(pend_data, 0);
            PackU64(pend_data, pend_res.order);
            PackU64(pend_data, 0);
            PackF64(pend_data, 0.0);
            PackF64(pend_data, 0.0);

            if (pend_ok && (pend_res.retcode == TRADE_RETCODE_DONE ||
                            pend_res.retcode == TRADE_RETCODE_PLACED ||
                            pend_res.retcode == TRADE_RETCODE_DONE_PARTIAL)) {
                SendOkData(h, pend_data, (uint)ArraySize(pend_data));
            } else {
                Print("MT5Bridge: OrderRemove failed retcode=", pend_res.retcode);
                SendResponse(h, -1, pend_data, (uint)ArraySize(pend_data));
            }
            return;
        }

        Print("MT5Bridge: position or order not found for ticket=", ticket);
        uchar not_found_data[];
        PackU32(not_found_data, 10036); // TRADE_RETCODE_POSITION_CLOSED
        PackU64(not_found_data, 0);
        PackU64(not_found_data, 0);
        PackU64(not_found_data, ticket);
        PackF64(not_found_data, 0.0);
        PackF64(not_found_data, 0.0);
        SendResponse(h, -1, not_found_data, (uint)ArraySize(not_found_data));
        return;
    }

    long pos_magic = PositionGetInteger(POSITION_MAGIC);
    if (req_magic > 0 && pos_magic != (long)req_magic) {
        Print("MT5Bridge: rejected close — position ticket ", ticket, " magic (", pos_magic,
              ") does not match requested magic (", req_magic, ")");
        SendError(h);
        return;
    } else if (InpEnforceMagicNumber && InpMagicNumber > 0 && pos_magic != InpMagicNumber) {
        Print("MT5Bridge: rejected close — position ticket ", ticket, " magic (", pos_magic,
              ") does not match EA magic (", InpMagicNumber, ")");
        SendError(h);
        return;
    } else if (InpMagicNumber > 0 && pos_magic != InpMagicNumber) {
        Print("MT5Bridge: warning — closing position ticket ", ticket, " magic (", pos_magic,
              ") does not match EA magic (", InpMagicNumber, ")");
    }

    string  sym   = PositionGetString(POSITION_SYMBOL);
    double  vol   = PositionGetDouble(POSITION_VOLUME);
    if (req_volume > 0.0 && req_volume <= vol) {
        vol = req_volume;
    }
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
    req.deviation= (req_deviation > 0) ? req_deviation : 10;
    req.position = ticket;
    req.magic    = (pos_magic > 0) ? pos_magic : InpMagicNumber;
    req.comment  = "bridge_close";
    req.type_filling = GetSymbolFillingMode(sym);

    MqlTradeResult res = {};
    bool ok = OrderSend(req, res);

    double exec_price = res.price;
    if (exec_price <= 0.0) {
        if (res.deal > 0 && HistoryDealSelect(res.deal)) {
            exec_price = HistoryDealGetDouble(res.deal, DEAL_PRICE);
        }
        // Do not substitute the requested close quote for a missing fill price.
        // The Rust executor will reconcile the closing deal or keep the
        // position unresolved.
    }

    uchar data[];
    PackU32(data, res.retcode);
    PackU64(data, res.deal);
    PackU64(data, res.order);
    PackU64(data, ticket);
    PackF64(data, res.volume);
    PackF64(data, exec_price);

    if (ok && (res.retcode == TRADE_RETCODE_DONE ||
               res.retcode == TRADE_RETCODE_PLACED ||
               res.retcode == TRADE_RETCODE_DONE_PARTIAL)) {
        SendOkData(h, data, (uint)ArraySize(data));
    } else {
        Print("MT5Bridge: OrderClose failed retcode=", res.retcode);
        SendResponse(h, -1, data, (uint)ArraySize(data));
    }
}

void SendModifyError(long h, uint retcode, ulong ticket, bool is_position) {
    uchar data[];
    PackU32(data, retcode);
    PackU64(data, 0);
    PackU64(data, (is_position ? 0 : ticket));
    PackU64(data, (is_position ? ticket : 0));
    PackF64(data, 0.0);
    PackF64(data, 0.0);
    SendResponse(h, -1, data, (uint)ArraySize(data));
}

void HandleOrderModify(long h, const uchar &payload[], uint len) {
    int off = 0;
    ulong ticket = 0;
    uchar sl_mode = WIRE_LEVEL_KEEP;
    double sl = 0.0;
    uchar tp_mode = WIRE_LEVEL_KEEP;
    double tp = 0.0;

    if (!SafeUnpackU64(payload, off, len, ticket)) {
        SendError(h);
        return;
    }
    if (!SafeUnpackU8(payload, off, len, sl_mode)) {
        SendError(h);
        return;
    }
    if (!SafeUnpackF64(payload, off, len, sl)) {
        SendError(h);
        return;
    }
    if (!SafeUnpackU8(payload, off, len, tp_mode)) {
        SendError(h);
        return;
    }
    if (!SafeUnpackF64(payload, off, len, tp)) {
        SendError(h);
        return;
    }

    ulong req_magic = 0;
    if ((uint)off + 8 <= len) {
        SafeUnpackU64(payload, off, len, req_magic);
    }

    MqlTradeRequest req = {};
    MqlTradeResult res = {};
    bool ok = false;
    bool is_position = false;

    if (PositionSelectByTicket(ticket)) {
        is_position = true;
        long pos_magic = PositionGetInteger(POSITION_MAGIC);
        if (req_magic > 0 && pos_magic != (long)req_magic) {
            Print("MT5Bridge: rejected modify — position ticket ", ticket, " magic (", pos_magic,
                  ") does not match requested magic (", req_magic, ")");
            SendModifyError(h, 10013, ticket, true);
            return;
        } else if (InpEnforceMagicNumber && InpMagicNumber > 0 && pos_magic != InpMagicNumber) {
            Print("MT5Bridge: rejected modify — position ticket ", ticket, " magic (", pos_magic,
                  ") does not match EA magic (", InpMagicNumber, ")");
            SendModifyError(h, 10013, ticket, true);
            return;
        } else if (InpMagicNumber > 0 && pos_magic != InpMagicNumber) {
            Print("MT5Bridge: warning — modifying position ticket ", ticket, " magic (", pos_magic,
                  ") does not match EA magic (", InpMagicNumber, ")");
        }

        string sym   = PositionGetString(POSITION_SYMBOL);
        ENUM_POSITION_TYPE ptype = (ENUM_POSITION_TYPE)PositionGetInteger(POSITION_TYPE);
        double bid = SymbolInfoDouble(sym, SYMBOL_BID);
        double ask = SymbolInfoDouble(sym, SYMBOL_ASK);
        double point = SymbolInfoDouble(sym, SYMBOL_POINT);
        double tick_size = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE);
        if (tick_size <= 0.0) tick_size = point;
        long stops_lvl = SymbolInfoInteger(sym, SYMBOL_TRADE_STOPS_LEVEL);
        double min_dist = (stops_lvl > 0 && point > 0.0) ? (stops_lvl * point) : 0.0;

        int sym_digits = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);
        double cur_sl = PositionGetDouble(POSITION_SL);
        double cur_tp = PositionGetDouble(POSITION_TP);
        const bool buy_side = (ptype == POSITION_TYPE_BUY);

        double target_sl = cur_sl;
        if (sl_mode == WIRE_LEVEL_CLEAR) {
            target_sl = 0.0;
        } else if (sl_mode == WIRE_LEVEL_SET) {
            target_sl = (sl > 0.0)
                ? SnapToTick(sl, tick_size, sym_digits, buy_side ? SNAP_DOWN : SNAP_UP)
                : 0.0;
            if (target_sl != sl) {
                Print("MT5Bridge: modify SL ", DoubleToString(sl, sym_digits),
                      " snapped to ", DoubleToString(target_sl, sym_digits),
                      " (tick size ", DoubleToString(tick_size, sym_digits), ")");
            }
        }

        double target_tp = cur_tp;
        if (tp_mode == WIRE_LEVEL_CLEAR) {
            target_tp = 0.0;
        } else if (tp_mode == WIRE_LEVEL_SET) {
            target_tp = (tp > 0.0)
                ? SnapToTick(tp, tick_size, sym_digits, buy_side ? SNAP_UP : SNAP_DOWN)
                : 0.0;
            if (target_tp != tp) {
                Print("MT5Bridge: modify TP ", DoubleToString(tp, sym_digits),
                      " snapped to ", DoubleToString(target_tp, sym_digits),
                      " (tick size ", DoubleToString(tick_size, sym_digits), ")");
            }
        }

        if (ptype == POSITION_TYPE_BUY) {
            if (target_sl > 0.0 && (bid - target_sl) < min_dist - 1e-8) {
                Print("MT5Bridge: modify Buy SL ", DoubleToString(target_sl, 5), " violates stops level from bid ", DoubleToString(bid, 5));
                SendModifyError(h, 10016, ticket, true); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
            if (target_tp > 0.0 && (target_tp - bid) < min_dist - 1e-8) {
                Print("MT5Bridge: modify Buy TP ", DoubleToString(target_tp, 5), " violates stops level from bid ", DoubleToString(bid, 5));
                SendModifyError(h, 10016, ticket, true); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
        } else if (ptype == POSITION_TYPE_SELL) {
            if (target_sl > 0.0 && (target_sl - ask) < min_dist - 1e-8) {
                Print("MT5Bridge: modify Sell SL ", DoubleToString(target_sl, 5), " violates stops level from ask ", DoubleToString(ask, 5));
                SendModifyError(h, 10016, ticket, true); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
            if (target_tp > 0.0 && (ask - target_tp) < min_dist - 1e-8) {
                Print("MT5Bridge: modify Sell TP ", DoubleToString(target_tp, 5), " violates stops level from ask ", DoubleToString(ask, 5));
                SendModifyError(h, 10016, ticket, true); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
        }

        req.action   = TRADE_ACTION_SLTP;
        req.position = ticket;
        req.symbol   = sym;
        req.sl       = target_sl;
        req.tp       = target_tp;
        req.magic    = (pos_magic > 0) ? pos_magic : InpMagicNumber;
        ok = OrderSend(req, res);
    } else if (OrderSelect(ticket)) {
        long ord_magic = OrderGetInteger(ORDER_MAGIC);
        if (req_magic > 0 && ord_magic != (long)req_magic) {
            Print("MT5Bridge: rejected modify — pending order ticket ", ticket, " magic (", ord_magic,
                  ") does not match requested magic (", req_magic, ")");
            SendModifyError(h, 10013, ticket, false);
            return;
        } else if (InpEnforceMagicNumber && InpMagicNumber > 0 && ord_magic != InpMagicNumber) {
            Print("MT5Bridge: rejected modify — pending order ticket ", ticket, " magic (", ord_magic,
                  ") does not match EA magic (", InpMagicNumber, ")");
            SendModifyError(h, 10013, ticket, false);
            return;
        } else if (InpMagicNumber > 0 && ord_magic != InpMagicNumber) {
            Print("MT5Bridge: warning — modifying pending order ticket ", ticket, " magic (", ord_magic,
                  ") does not match EA magic (", InpMagicNumber, ")");
        }

        string sym   = OrderGetString(ORDER_SYMBOL);
        ENUM_ORDER_TYPE otype = (ENUM_ORDER_TYPE)OrderGetInteger(ORDER_TYPE);
        double ord_price = OrderGetDouble(ORDER_PRICE_OPEN);
        double point = SymbolInfoDouble(sym, SYMBOL_POINT);
        double tick_size = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE);
        if (tick_size <= 0.0) tick_size = point;
        long stops_lvl = SymbolInfoInteger(sym, SYMBOL_TRADE_STOPS_LEVEL);
        double min_dist = (stops_lvl > 0 && point > 0.0) ? (stops_lvl * point) : 0.0;

        int sym_digits = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);
        double cur_sl = OrderGetDouble(ORDER_SL);
        double cur_tp = OrderGetDouble(ORDER_TP);
        const bool buy_side = IsBuySideOrder(otype);

        double target_sl = cur_sl;
        if (sl_mode == WIRE_LEVEL_CLEAR) {
            target_sl = 0.0;
        } else if (sl_mode == WIRE_LEVEL_SET) {
            target_sl = (sl > 0.0)
                ? SnapToTick(sl, tick_size, sym_digits, buy_side ? SNAP_DOWN : SNAP_UP)
                : 0.0;
            if (target_sl != sl) {
                Print("MT5Bridge: modify pending SL ", DoubleToString(sl, sym_digits),
                      " snapped to ", DoubleToString(target_sl, sym_digits),
                      " (tick size ", DoubleToString(tick_size, sym_digits), ")");
            }
        }

        double target_tp = cur_tp;
        if (tp_mode == WIRE_LEVEL_CLEAR) {
            target_tp = 0.0;
        } else if (tp_mode == WIRE_LEVEL_SET) {
            target_tp = (tp > 0.0)
                ? SnapToTick(tp, tick_size, sym_digits, buy_side ? SNAP_UP : SNAP_DOWN)
                : 0.0;
            if (target_tp != tp) {
                Print("MT5Bridge: modify pending TP ", DoubleToString(tp, sym_digits),
                      " snapped to ", DoubleToString(target_tp, sym_digits),
                      " (tick size ", DoubleToString(tick_size, sym_digits), ")");
            }
        }

        if (tick_size > 0.0) {
            if (target_sl > 0.0 && MathAbs(target_sl / tick_size - MathRound(target_sl / tick_size)) > 1e-4) {
                Print("MT5Bridge: modify pending SL ", DoubleToString(target_sl, 5), " not aligned to tick size ", DoubleToString(tick_size, 5));
                SendModifyError(h, 10015, ticket, false); // TRADE_RETCODE_INVALID_PRICE
                return;
            }
            if (target_tp > 0.0 && MathAbs(target_tp / tick_size - MathRound(target_tp / tick_size)) > 1e-4) {
                Print("MT5Bridge: modify pending TP ", DoubleToString(target_tp, 5), " not aligned to tick size ", DoubleToString(tick_size, 5));
                SendModifyError(h, 10015, ticket, false); // TRADE_RETCODE_INVALID_PRICE
                return;
            }
        }

        if (otype == ORDER_TYPE_BUY_LIMIT || otype == ORDER_TYPE_BUY_STOP) {
            if (target_sl > 0.0 && (ord_price - target_sl) < min_dist - 1e-8) {
                Print("MT5Bridge: modify pending Buy SL violates stops level relative to price");
                SendModifyError(h, 10016, ticket, false); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
            if (target_tp > 0.0 && (target_tp - ord_price) < min_dist - 1e-8) {
                Print("MT5Bridge: modify pending Buy TP violates stops level relative to price");
                SendModifyError(h, 10016, ticket, false); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
        } else if (otype == ORDER_TYPE_SELL_LIMIT || otype == ORDER_TYPE_SELL_STOP) {
            if (target_sl > 0.0 && (target_sl - ord_price) < min_dist - 1e-8) {
                Print("MT5Bridge: modify pending Sell SL violates stops level relative to price");
                SendModifyError(h, 10016, ticket, false); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
            if (target_tp > 0.0 && (ord_price - target_tp) < min_dist - 1e-8) {
                Print("MT5Bridge: modify pending Sell TP violates stops level relative to price");
                SendModifyError(h, 10016, ticket, false); // TRADE_RETCODE_INVALID_STOPS
                return;
            }
        }

        req.action      = TRADE_ACTION_MODIFY;
        req.order       = ticket;
        req.symbol      = sym;
        req.price       = SnapToTick(ord_price, tick_size, sym_digits);
        req.sl          = target_sl;
        req.tp          = target_tp;
        req.type_time   = (ENUM_ORDER_TYPE_TIME)OrderGetInteger(ORDER_TYPE_TIME);
        req.expiration  = (datetime)OrderGetInteger(ORDER_TIME_EXPIRATION);
        req.magic       = (ord_magic > 0) ? ord_magic : InpMagicNumber;
        ok = OrderSend(req, res);
    } else {
        Print("MT5Bridge: ticket ", ticket, " not found for modify");
        SendModifyError(h, 10036, ticket, false); // TRADE_RETCODE_POSITION_CLOSED
        return;
    }

    uchar data[];
    PackU32(data, res.retcode);
    PackU64(data, res.deal);
    PackU64(data, res.order);
    PackU64(data, (is_position ? ticket : 0));
    PackF64(data, res.volume);
    PackF64(data, res.price);

    if (ok && (res.retcode == TRADE_RETCODE_DONE ||
               res.retcode == TRADE_RETCODE_PLACED ||
               res.retcode == TRADE_RETCODE_DONE_PARTIAL)) {
        SendOkData(h, data, (uint)ArraySize(data));
    } else {
        Print("MT5Bridge: OrderModify failed retcode=", res.retcode);
        SendResponse(h, -1, data, (uint)ArraySize(data));
    }
}

// MQL5 wire intermediates (matches Mt5SymInfo and Mt5Tick in mt5_bridge.h).
void HandleSymbolInfoFull(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym)) { SendError(h); return; }

    if (!EnsureSymbolAvailable(sym)) {
        Print("MT5Bridge: symbol '", sym, "' not available in Market Watch");
        SendError(h);
        return;
    }

    double point      = SymbolInfoDouble(sym, SYMBOL_POINT);
    double tick_value = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_VALUE);
    double tick_size  = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE);
    if (tick_size <= 0.0) tick_size = point;
    double lot_step   = SymbolInfoDouble(sym, SYMBOL_VOLUME_STEP);
    double min_lot    = SymbolInfoDouble(sym, SYMBOL_VOLUME_MIN);
    double max_lot    = SymbolInfoDouble(sym, SYMBOL_VOLUME_MAX);
    int    digits     = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);

    if (point <= 0.0) {
        Print("MT5Bridge: invalid symbol properties for '", sym, "'");
        SendError(h);
        return;
    }

    MqlTick tick;
    double spread = 0.0;
    double bid = 0.0;
    double ask = 0.0;
    if (SymbolInfoTick(sym, tick) && point > 0.0) {
        spread = (tick.ask - tick.bid) / point;
        bid = tick.bid;
        ask = tick.ask;
    }

    // Serialise as packed wire format: 7×f64 + i32 + 2×f64 = 76 bytes (matches Mt5SymInfo in mt5_bridge.h).
    uchar data[];
    PackF64(data, point);
    PackF64(data, tick_value);
    PackF64(data, tick_size);
    PackF64(data, lot_step);
    PackF64(data, min_lot);
    PackF64(data, max_lot);
    PackF64(data, spread);
    PackI32(data, digits);
    PackF64(data, bid);
    PackF64(data, ask);
    SendOkData(h, data, (uint)ArraySize(data));
}

void HandleSymbolInfoTick(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym)) { SendError(h); return; }

    if (!EnsureSymbolAvailable(sym)) {
        Print("MT5Bridge: symbol '", sym, "' not available in Market Watch");
        SendError(h);
        return;
    }

    MqlTick tick;
    if (!SymbolInfoTick(sym, tick)) { SendError(h); return; }

    long offset_ms = GetServerUtcOffset((datetime)tick.time) * 1000;
    long tick_ms = (long)tick.time_msc;
    if (tick_ms <= 0) {
        tick_ms = (long)tick.time * 1000;
    }

    // Serialise as packed wire format: i64 (time_msc UTC) + 3×f64 + u64 + u32 = 44 bytes
    // (matches Mt5Tick in mt5_bridge.h).
    uchar data[];
    PackI64(data, tick_ms - offset_ms);
    PackF64(data, tick.bid);
    PackF64(data, tick.ask);
    PackF64(data, tick.last);
    PackU64(data, tick.volume);
    PackU32(data, tick.flags);
    SendOkData(h, data, (uint)ArraySize(data));
}

void HandlePositionsGet(long h, const uchar &payload[], uint len) {
    int off = 0;
    ulong magic_filter = 0;
    string symbol_filter = "";
    int max_items = 0;

    SafeUnpackU64(payload, off, len, magic_filter);
    SafeUnpackStr(payload, off, len, symbol_filter);
    SafeUnpackI32(payload, off, len, max_items);
    if (max_items <= 0) max_items = 1000;

    int total = PositionsTotal();
    uchar data[];
    int count = 0;
    int total_matching = 0;

    for (int i = 0; i < total; i++) {
        ulong ticket = PositionGetTicket(i);
        if (ticket == 0) continue;

        ulong pos_magic = (ulong)PositionGetInteger(POSITION_MAGIC);
        if (magic_filter > 0 && pos_magic != magic_filter) continue;

        string sym = PositionGetString(POSITION_SYMBOL);
        if (StringLen(symbol_filter) > 0 && sym != symbol_filter) continue;

        total_matching++;
        if (count >= max_items) continue;

        long pos_time     = PositionGetInteger(POSITION_TIME);
        if (InpConvertToUTC) pos_time -= GetServerUtcOffset((datetime)pos_time);
        int pos_type      = (int)PositionGetInteger(POSITION_TYPE);
        double volume     = PositionGetDouble(POSITION_VOLUME);
        double price_open = PositionGetDouble(POSITION_PRICE_OPEN);
        double sl         = PositionGetDouble(POSITION_SL);
        double tp         = PositionGetDouble(POSITION_TP);
        double price_curr = PositionGetDouble(POSITION_PRICE_CURRENT);
        double profit     = PositionGetDouble(POSITION_PROFIT);
        double swap       = PositionGetDouble(POSITION_SWAP);
        string comment    = PositionGetString(POSITION_COMMENT);

        PackU64(data, ticket);
        PackI64(data, pos_time);
        PackI32(data, pos_type);
        PackU64(data, pos_magic);
        PackF64(data, volume);
        PackF64(data, price_open);
        PackF64(data, sl);
        PackF64(data, tp);
        PackF64(data, price_curr);
        PackF64(data, profit);
        PackF64(data, swap);
        PackFixedString32(data, sym);
        PackFixedString32(data, comment);

        count++;
    }

    SendResponse(h, total_matching, data, (uint)ArraySize(data));
}

void HandleOrdersGet(long h, const uchar &payload[], uint len) {
    int off = 0;
    ulong magic_filter = 0;
    string symbol_filter = "";
    int max_items = 0;

    SafeUnpackU64(payload, off, len, magic_filter);
    SafeUnpackStr(payload, off, len, symbol_filter);
    SafeUnpackI32(payload, off, len, max_items);
    if (max_items <= 0) max_items = 1000;

    int total = OrdersTotal();
    uchar data[];
    int count = 0;
    int total_matching = 0;

    for (int i = 0; i < total; i++) {
        ulong ticket = OrderGetTicket(i);
        if (ticket == 0) continue;

        ulong ord_magic = (ulong)OrderGetInteger(ORDER_MAGIC);
        if (magic_filter > 0 && ord_magic != magic_filter) continue;

        string sym = OrderGetString(ORDER_SYMBOL);
        if (StringLen(symbol_filter) > 0 && sym != symbol_filter) continue;

        total_matching++;
        if (count >= max_items) continue;

        long ord_time     = OrderGetInteger(ORDER_TIME_SETUP);
        if (InpConvertToUTC) ord_time -= GetServerUtcOffset((datetime)ord_time);
        int ord_type      = (int)OrderGetInteger(ORDER_TYPE);
        double vol_init   = OrderGetDouble(ORDER_VOLUME_INITIAL);
        double vol_curr   = OrderGetDouble(ORDER_VOLUME_CURRENT);
        double price_open = OrderGetDouble(ORDER_PRICE_OPEN);
        double sl         = OrderGetDouble(ORDER_SL);
        double tp         = OrderGetDouble(ORDER_TP);
        double price_curr = OrderGetDouble(ORDER_PRICE_CURRENT);
        string comment    = OrderGetString(ORDER_COMMENT);

        PackU64(data, ticket);
        PackI64(data, ord_time);
        PackI32(data, ord_type);
        PackU64(data, ord_magic);
        PackF64(data, vol_init);
        PackF64(data, vol_curr);
        PackF64(data, price_open);
        PackF64(data, sl);
        PackF64(data, tp);
        PackF64(data, price_curr);
        PackFixedString32(data, sym);
        PackFixedString32(data, comment);

        count++;
    }

    SendResponse(h, total_matching, data, (uint)ArraySize(data));
}

// Completed trade deals (MT5 history) in [from, to] (UTC seconds), filtered by magic / symbol.
// Request layout (must match DealsGet in bridge_dll/mt5_bridge.cpp):
//     i64 from, i64 to, u64 magic_filter, str symbol_filter, i32 max_items
// Response: count + count * 152-byte Mt5Deal records (layout in bridge_dll/mt5_bridge.h / src/ffi.rs).
// Only BUY/SELL deals are returned. If more than max_items match, output stops at max_items and the
// client treats count == max_items as "possibly truncated" (it never silently accepts a partial history).
void HandleDealsGet(long h, const uchar &payload[], uint len) {
    int off = 0;
    long from_utc = 0, to_utc = 0;
    ulong magic_filter = 0;
    string symbol_filter = "";
    int max_items = 0;

    if (!SafeUnpackI64(payload, off, len, from_utc) ||
        !SafeUnpackI64(payload, off, len, to_utc)) {
        SendError(h);
        return;
    }
    SafeUnpackU64(payload, off, len, magic_filter);
    SafeUnpackStr(payload, off, len, symbol_filter);
    SafeUnpackI32(payload, off, len, max_items);
    if (max_items <= 0) max_items = 1000;
    if (to_utc < from_utc) { SendError(h); return; }

    long from_offset = GetServerUtcOffset((datetime)from_utc);
    long to_offset   = GetServerUtcOffset((datetime)to_utc);

    if (!HistorySelect((datetime)(from_utc + from_offset), (datetime)(to_utc + to_offset))) {
        Print("MT5Bridge: HistorySelect failed for deals query, error=", GetLastError());
        SendError(h);
        return;
    }

    int total = HistoryDealsTotal();
    uchar data[];
    int count = 0;
    int total_matching = 0;

    for (int i = 0; i < total; i++) {
        ulong ticket = HistoryDealGetTicket(i);
        if (ticket == 0) continue;

        long deal_type = HistoryDealGetInteger(ticket, DEAL_TYPE);
        if (deal_type != DEAL_TYPE_BUY && deal_type != DEAL_TYPE_SELL) continue;  // skip balance/credit/etc.

        ulong deal_magic = (ulong)HistoryDealGetInteger(ticket, DEAL_MAGIC);
        if (magic_filter > 0 && deal_magic != magic_filter) continue;

        string sym = HistoryDealGetString(ticket, DEAL_SYMBOL);
        if (StringLen(symbol_filter) > 0 && sym != symbol_filter) continue;

        total_matching++;
        if (count >= max_items) continue;

        long deal_time = HistoryDealGetInteger(ticket, DEAL_TIME);
        if (InpConvertToUTC) deal_time -= GetServerUtcOffset((datetime)deal_time);

        PackU64(data, ticket);
        PackU64(data, (ulong)HistoryDealGetInteger(ticket, DEAL_ORDER));
        PackU64(data, (ulong)HistoryDealGetInteger(ticket, DEAL_POSITION_ID));
        PackI64(data, deal_time);
        PackI32(data, (deal_type == DEAL_TYPE_SELL) ? 1 : 0);
        PackI32(data, (int)HistoryDealGetInteger(ticket, DEAL_ENTRY));
        PackU64(data, deal_magic);
        PackF64(data, HistoryDealGetDouble(ticket, DEAL_VOLUME));
        PackF64(data, HistoryDealGetDouble(ticket, DEAL_PRICE));
        PackF64(data, HistoryDealGetDouble(ticket, DEAL_COMMISSION));
        PackF64(data, HistoryDealGetDouble(ticket, DEAL_SWAP));
        PackF64(data, HistoryDealGetDouble(ticket, DEAL_PROFIT));
        PackFixedString32(data, sym);
        PackFixedString32(data, HistoryDealGetString(ticket, DEAL_COMMENT));

        count++;
    }

    SendResponse(h, total_matching, data, (uint)ArraySize(data));
}

// ── Subscription management (protocol v5+) ──────────────────────────────────

struct TickSubscription {
    string symbol;
    bool   enabled;
    long   last_time_msc;
    double last_bid;
    double last_ask;
    double last_price;
    ulong  last_volume;
    uint   last_flags;
};

TickSubscription g_tick_subscriptions[];
bool             g_trade_subscription = false;
string           g_book_subscriptions[];

void HandleSubscribeTicks(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym) || StringLen(sym) == 0) {
        SendError(h);
        return;
    }
    if (!EnsureSymbolAvailable(sym)) {
        SendError(h);
        return;
    }

    int total = ArraySize(g_tick_subscriptions);
    int idx = -1;
    for (int i = 0; i < total; i++) {
        if (g_tick_subscriptions[i].symbol == sym) {
            idx = i;
            break;
        }
    }
    if (idx < 0) {
        ArrayResize(g_tick_subscriptions, total + 1);
        idx = total;
        g_tick_subscriptions[idx].symbol        = sym;
        g_tick_subscriptions[idx].last_time_msc = 0;
        g_tick_subscriptions[idx].last_bid      = 0.0;
        g_tick_subscriptions[idx].last_ask      = 0.0;
        g_tick_subscriptions[idx].last_price    = 0.0;
        g_tick_subscriptions[idx].last_volume   = 0;
        g_tick_subscriptions[idx].last_flags    = 0;
    }
    g_tick_subscriptions[idx].enabled = true;
    SendOk(h);
    Print("MT5Bridge: subscribed to ticks for '", sym, "'");
}

void HandleUnsubscribeTicks(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym) || StringLen(sym) == 0) {
        SendError(h);
        return;
    }
    int total = ArraySize(g_tick_subscriptions);
    for (int i = 0; i < total; i++) {
        if (g_tick_subscriptions[i].symbol == sym) {
            g_tick_subscriptions[i].enabled = false;
            break;
        }
    }
    SendOk(h);
    Print("MT5Bridge: unsubscribed from ticks for '", sym, "'");
}

void HandleSubscribeTrade(long h) {
    g_trade_subscription = true;
    SendOk(h);
    Print("MT5Bridge: subscribed to trade events");
}

void HandleUnsubscribeTrade(long h) {
    g_trade_subscription = false;
    SendOk(h);
    Print("MT5Bridge: unsubscribed from trade events");
}

void HandleSubscribeBook(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym) || StringLen(sym) == 0) {
        SendError(h);
        return;
    }
    EnsureSymbolAvailable(sym);
    MarketBookAdd(sym);
    int total = ArraySize(g_book_subscriptions);
    bool found = false;
    for (int i = 0; i < total; i++) {
        if (g_book_subscriptions[i] == sym) { found = true; break; }
    }
    if (!found) {
        ArrayResize(g_book_subscriptions, total + 1);
        g_book_subscriptions[total] = sym;
    }
    SendOk(h);
}

void HandleUnsubscribeBook(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym) || StringLen(sym) == 0) {
        SendError(h);
        return;
    }
    MarketBookRelease(sym);
    int total = ArraySize(g_book_subscriptions);
    for (int i = 0; i < total; i++) {
        if (g_book_subscriptions[i] == sym) {
            g_book_subscriptions[i] = "";
            break;
        }
    }
    SendOk(h);
}

struct BarSubscription {
    string symbol;
    int    timeframe;
    bool   enabled;
    long   last_bar_time;
};

BarSubscription g_bar_subscriptions[];

void HandleSubscribeBars(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym) || StringLen(sym) == 0) {
        SendError(h);
        return;
    }
    int tf = 0;
    if (!SafeUnpackI32(payload, off, len, tf)) {
        SendError(h);
        return;
    }
    if (!EnsureSymbolAvailable(sym)) {
        SendError(h);
        return;
    }

    int total = ArraySize(g_bar_subscriptions);
    int idx = -1;
    for (int i = 0; i < total; i++) {
        if (g_bar_subscriptions[i].symbol == sym && g_bar_subscriptions[i].timeframe == tf) {
            idx = i;
            break;
        }
    }
    if (idx < 0) {
        ArrayResize(g_bar_subscriptions, total + 1);
        idx = total;
        g_bar_subscriptions[idx].symbol        = sym;
        g_bar_subscriptions[idx].timeframe     = tf;
        datetime cur_time = iTime(sym, (ENUM_TIMEFRAMES)tf, 0);
        g_bar_subscriptions[idx].last_bar_time = (long)cur_time;
    }
    g_bar_subscriptions[idx].enabled = true;
    SendOk(h);
    Print("MT5Bridge: subscribed to bars for '", sym, "' tf=", tf);
}

void HandleUnsubscribeBars(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    if (!SafeUnpackStr(payload, off, len, sym) || StringLen(sym) == 0) {
        SendError(h);
        return;
    }
    int tf = 0;
    if (!SafeUnpackI32(payload, off, len, tf)) {
        SendError(h);
        return;
    }
    int total = ArraySize(g_bar_subscriptions);
    for (int i = 0; i < total; i++) {
        if (g_bar_subscriptions[i].symbol == sym && g_bar_subscriptions[i].timeframe == tf) {
            g_bar_subscriptions[i].enabled = false;
            break;
        }
    }
    SendOk(h);
    Print("MT5Bridge: unsubscribed from bars for '", sym, "' tf=", tf);
}

void CheckSubscribedBars() {
    if (g_client == INVALID_HANDLE || !g_authenticated) return;
    int total = ArraySize(g_bar_subscriptions);
    for (int i = 0; i < total; i++) {
        if (!g_bar_subscriptions[i].enabled) continue;
        string sym = g_bar_subscriptions[i].symbol;
        int tf = g_bar_subscriptions[i].timeframe;
        datetime cur_time = iTime(sym, (ENUM_TIMEFRAMES)tf, 0);
        if (cur_time == 0) continue;
        if (g_bar_subscriptions[i].last_bar_time == 0) {
            g_bar_subscriptions[i].last_bar_time = (long)cur_time;
            continue;
        }
        if ((long)cur_time > g_bar_subscriptions[i].last_bar_time) {
            MqlRates rates[];
            ArraySetAsSeries(rates, true);
            if (CopyRates(sym, (ENUM_TIMEFRAMES)tf, 1, 1, rates) == 1) {
                datetime bar_time = rates[0].time;
                if (bar_time == 0) {
                    bar_time = (datetime)(ServerTimeMsc() / 1000);
                } else if (InpConvertToUTC) {
                    bar_time = ConvertServerTimeToUTC(bar_time);
                }

                uchar payload[];
                PackFixedString32(payload, sym);
                PackI64(payload, (long)bar_time);
                PackF64(payload, rates[0].open);
                PackF64(payload, rates[0].high);
                PackF64(payload, rates[0].low);
                PackF64(payload, rates[0].close);
                PackI64(payload, rates[0].tick_volume);
                PackI32(payload, tf);
                PackI32(payload, rates[0].spread);
                PackU64(payload, ++g_event_seq);

                if (!SendEvent(g_client, EVENT_BAR, payload, 96)) {
                    ResetPipeServer("failed to write bar event to pipe");
                    return;
                }
                FlushFileBuffers(g_client);
            }
            g_bar_subscriptions[i].last_bar_time = (long)cur_time;
        }
    }
}

// ── Event queues (bounded ring-buffers) ──────────────────────────────────────

struct QueuedTickEvent {
    string   symbol;
    long     time_msc;
    double   bid;
    double   ask;
    double   last;
    ulong    volume;
    uint     flags;
};

#define TICK_QUEUE_CAPACITY 2048
QueuedTickEvent g_tick_queue[TICK_QUEUE_CAPACITY];
int g_tick_queue_head       = 0;
int g_tick_queue_tail       = 0;
int g_tick_queue_count      = 0;
ulong g_ticks_dropped_total = 0;

void EnqueueTick(const string sym, const MqlTick &tick) {
    if (g_tick_queue_count >= TICK_QUEUE_CAPACITY) {
        g_tick_queue_tail = (g_tick_queue_tail + 1) % TICK_QUEUE_CAPACITY;
        g_tick_queue_count--;
        g_ticks_dropped_total++;
        if (g_ticks_dropped_total % 100 == 1) {
            Print("MT5Bridge: warning — tick queue full (capacity=", TICK_QUEUE_CAPACITY,
                  "), dropped oldest tick (total dropped=", g_ticks_dropped_total, ")");
        }
    }
    long tm = tick.time_msc;
    if (tm == 0) {
        tm = (long)tick.time * 1000;
    }
    if (tm == 0) {
        tm = ServerTimeMsc();
    } else if (InpConvertToUTC) {
        tm -= ((long)TimeTradeServer() * 1000 - ServerTimeMsc());
    }
    g_tick_queue[g_tick_queue_head].symbol   = sym;
    g_tick_queue[g_tick_queue_head].time_msc = tm;
    g_tick_queue[g_tick_queue_head].bid      = tick.bid;
    g_tick_queue[g_tick_queue_head].ask      = tick.ask;
    g_tick_queue[g_tick_queue_head].last     = tick.last;
    g_tick_queue[g_tick_queue_head].volume   = tick.volume;
    g_tick_queue[g_tick_queue_head].flags    = tick.flags;
    g_tick_queue_head = (g_tick_queue_head + 1) % TICK_QUEUE_CAPACITY;
    g_tick_queue_count++;
}

bool CheckAndUpdateTick(int sub_idx, const MqlTick &tick) {
    if (tick.bid <= 0.0 && tick.ask <= 0.0) return false;
    bool time_dup  = (tick.time_msc == g_tick_subscriptions[sub_idx].last_time_msc);
    bool bid_dup   = (MathAbs(tick.bid - g_tick_subscriptions[sub_idx].last_bid) < 1e-9);
    bool ask_dup   = (MathAbs(tick.ask - g_tick_subscriptions[sub_idx].last_ask) < 1e-9);
    bool last_dup  = (MathAbs(tick.last - g_tick_subscriptions[sub_idx].last_price) < 1e-9);
    bool vol_dup   = (tick.volume == g_tick_subscriptions[sub_idx].last_volume);
    bool flags_dup = (tick.flags == g_tick_subscriptions[sub_idx].last_flags);

    if (time_dup && bid_dup && ask_dup && last_dup && vol_dup && flags_dup)
        return false;

    g_tick_subscriptions[sub_idx].last_time_msc = tick.time_msc;
    g_tick_subscriptions[sub_idx].last_bid      = tick.bid;
    g_tick_subscriptions[sub_idx].last_ask      = tick.ask;
    g_tick_subscriptions[sub_idx].last_price    = tick.last;
    g_tick_subscriptions[sub_idx].last_volume   = tick.volume;
    g_tick_subscriptions[sub_idx].last_flags    = tick.flags;
    return true;
}

void FlushTickQueue() {
    if (g_client == INVALID_HANDLE || !g_authenticated || g_tick_queue_count == 0)
        return;

    int flushed = 0;
    while (g_tick_queue_count > 0) {
        QueuedTickEvent qe = g_tick_queue[g_tick_queue_tail];
        g_tick_queue_tail = (g_tick_queue_tail + 1) % TICK_QUEUE_CAPACITY;
        g_tick_queue_count--;

        uchar payload[];
        PackFixedString32(payload, qe.symbol);
        PackI64(payload, qe.time_msc);
        PackF64(payload, qe.bid);
        PackF64(payload, qe.ask);
        PackF64(payload, qe.last);
        PackU64(payload, qe.volume);
        PackU32(payload, qe.flags);
        PackI32(payload, 0); // _pad
        PackU64(payload, ++g_event_seq);

        if (!SendEvent(g_client, EVENT_TICK, payload, 88)) {
            ResetPipeServer("failed to write tick event to pipe");
            return;
        }
        flushed++;
    }
    if (flushed > 0) {
        FlushFileBuffers(g_client);
    }
}

void CheckSubscribedSymbols() {
    int total = ArraySize(g_tick_subscriptions);
    for (int i = 0; i < total; i++) {
        if (!g_tick_subscriptions[i].enabled) continue;
        string sym = g_tick_subscriptions[i].symbol;
        MqlTick tick;
        if (SymbolInfoTick(sym, tick)) {
            if (CheckAndUpdateTick(i, tick)) {
                EnqueueTick(sym, tick);
            }
        }
    }
}

struct QueuedTradeEvent {
    ulong  deal;
    ulong  order;
    ulong  position;
    long   time;
    int    trans_type;
    int    order_type;
    double price;
    double volume;
    double sl;
    double tp;
    string symbol;
    string comment;
    int    deal_entry;
    int    deal_type;
    ulong  deal_magic;
    ulong  deal_position_id;
    int    deal_reason;
};

#define TRADE_QUEUE_CAPACITY 512
QueuedTradeEvent g_trade_queue[TRADE_QUEUE_CAPACITY];
int g_trade_queue_head       = 0;
int g_trade_queue_tail       = 0;
int g_trade_queue_count      = 0;
ulong g_trades_dropped_total = 0;

void EnqueueTrade(ulong deal, ulong order, ulong pos, long time, int trans_type,
                  int otype, double price, double vol, double sl, double tp,
                  const string sym, const string cmt,
                  int deal_entry, int deal_type, ulong deal_magic,
                  ulong deal_position_id, int deal_reason) {
    if (g_trade_queue_count >= TRADE_QUEUE_CAPACITY) {
        g_trade_queue_tail = (g_trade_queue_tail + 1) % TRADE_QUEUE_CAPACITY;
        g_trade_queue_count--;
        g_trades_dropped_total++;
        Print("MT5Bridge: CRITICAL — trade event queue full (capacity=", TRADE_QUEUE_CAPACITY,
              "), dropped oldest trade event (total dropped=", g_trades_dropped_total, ")");
    }
    g_trade_queue[g_trade_queue_head].deal             = deal;
    g_trade_queue[g_trade_queue_head].order            = order;
    g_trade_queue[g_trade_queue_head].position         = pos;
    g_trade_queue[g_trade_queue_head].time             = time;
    g_trade_queue[g_trade_queue_head].trans_type       = trans_type;
    g_trade_queue[g_trade_queue_head].order_type       = otype;
    g_trade_queue[g_trade_queue_head].price            = price;
    g_trade_queue[g_trade_queue_head].volume           = vol;
    g_trade_queue[g_trade_queue_head].sl               = sl;
    g_trade_queue[g_trade_queue_head].tp               = tp;
    g_trade_queue[g_trade_queue_head].symbol           = sym;
    g_trade_queue[g_trade_queue_head].comment          = cmt;
    g_trade_queue[g_trade_queue_head].deal_entry       = deal_entry;
    g_trade_queue[g_trade_queue_head].deal_type        = deal_type;
    g_trade_queue[g_trade_queue_head].deal_magic       = deal_magic;
    g_trade_queue[g_trade_queue_head].deal_position_id = deal_position_id;
    g_trade_queue[g_trade_queue_head].deal_reason      = deal_reason;
    g_trade_queue_head = (g_trade_queue_head + 1) % TRADE_QUEUE_CAPACITY;
    g_trade_queue_count++;
}

void FlushTradeQueue() {
    if (g_client == INVALID_HANDLE || !g_authenticated || g_trade_queue_count == 0)
        return;

    int flushed = 0;
    while (g_trade_queue_count > 0) {
        QueuedTradeEvent qe = g_trade_queue[g_trade_queue_tail];
        g_trade_queue_tail = (g_trade_queue_tail + 1) % TRADE_QUEUE_CAPACITY;
        g_trade_queue_count--;

        uchar payload[];
        PackU64(payload, qe.deal);
        PackU64(payload, qe.order);
        PackU64(payload, qe.position);
        PackI64(payload, qe.time);
        PackI32(payload, qe.trans_type);
        PackI32(payload, qe.order_type);
        PackF64(payload, qe.price);
        PackF64(payload, qe.volume);
        PackF64(payload, qe.sl);
        PackF64(payload, qe.tp);
        PackFixedString32(payload, qe.symbol);
        PackFixedString32(payload, qe.comment);
        PackI32(payload, qe.deal_entry);
        PackI32(payload, qe.deal_type);
        PackU64(payload, qe.deal_magic);
        PackU64(payload, qe.deal_position_id);
        PackI32(payload, qe.deal_reason);
        PackI32(payload, 0); // _pad
        PackU64(payload, ++g_event_seq);

        if (!SendEvent(g_client, EVENT_TRADE, payload, 176)) {
            ResetPipeServer("failed to write trade event to pipe");
            return;
        }
        flushed++;
    }
    if (flushed > 0) {
        FlushFileBuffers(g_client);
    }
}

void HandlePing(long h) {
    long server_time = (long)TimeTradeServer();
    ulong monotonic_us = GetMicrosecondCount();
    uint proto = PROTOCOL_VERSION;
    uchar data[];
    PackI64(data, server_time);
    PackU64(data, monotonic_us);
    PackU32(data, proto);
    SendOkData(h, data, (uint)ArraySize(data));
}

// ── Symbol universe discovery handlers (protocol v8+) ───────────────────────

// Returns the total number of symbols available in Market Watch.
// Response: status = total count, no payload.
void HandleSymbolsTotal(long h) {
    // SymbolsTotal() is not available in this MT5 build.
    // Full implementation for real terminals:
    /*
    int total = SymbolsTotal();
    uchar empty[1];
    SendResponse(h, total, empty, 0);
    */
    uchar empty[1];
    SendResponse(h, 0, empty, 0);
}

// Returns a page of symbols as packed entries:
//   [u32 name_len][name bytes][Mt5SymInfo 76 bytes] repeated
// Request layout (must match SymbolsGet in bridge_dll/mt5_bridge.cpp):
//     u32 page, u32 page_size
// Response: status = total count, payload = packed entries for this page.
void HandleSymbolsGet(long h, const uchar &payload[], uint len) {
    // Symbol discovery (SymbolsTotal/SymbolName/SymbolInfoDouble/SymbolInfoInteger/SymbolInfoTick)
    // is not available in this MT5 build. Full implementation preserved below for real terminals:
    /*
    int off = 0;
    uint page = 0, page_size = 0;
    if (!SafeUnpackU32(payload, off, len, page) ||
        !SafeUnpackU32(payload, off, len, page_size)) {
        SendError(h);
        return;
    }
    if (page_size == 0) page_size = 1000;

    int total = SymbolsTotal();
    uchar data[];
    int start = (int)page * (int)page_size;
    int end = MathMin(start + (int)page_size, total);
    if (start >= total) {
        uchar empty[1];
        SendResponse(h, total, empty, 0);
        return;
    }

    for (int i = start; i < end; i++) {
        string sym = SymbolName(i);
        if (StringLen(sym) == 0) continue;

        if (!EnsureSymbolAvailable(sym)) continue;

        double point      = SymbolInfoDouble(sym, SYMBOL_POINT);
        double tick_value = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_VALUE);
        double tick_size  = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE);
        if (tick_size <= 0.0) tick_size = point;
        double lot_step   = SymbolInfoDouble(sym, SYMBOL_VOLUME_STEP);
        double min_lot    = SymbolInfoDouble(sym, SYMBOL_VOLUME_MIN);
        double max_lot    = SymbolInfoDouble(sym, SYMBOL_VOLUME_MAX);
        int    digits     = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);

        if (point <= 0.0) continue;

        MqlTick tick;
        double spread = 0.0;
        double bid = 0.0;
        double ask = 0.0;
        if (SymbolInfoTick(sym, tick) && point > 0.0) {
            spread = (tick.ask - tick.bid) / point;
            bid = tick.bid;
            ask = tick.ask;
        }

        PackStr(data, sym);
        PackF64(data, point);
        PackF64(data, tick_value);
        PackF64(data, tick_size);
        PackF64(data, lot_step);
        PackF64(data, min_lot);
        PackF64(data, max_lot);
        PackF64(data, spread);
        PackI32(data, digits);
        PackF64(data, bid);
        PackF64(data, ask);

        PackU64(data, ++g_event_seq);
    }

    SendResponse(h, total, data, (uint)ArraySize(data));
    */
    uchar empty[1];
    SendResponse(h, 0, empty, 0);
}

// Selects or deselects a symbol in Market Watch.
// Request layout (must match SymbolSelect in bridge_dll/mt5_bridge.cpp):
//     str symbol, i32 enable (0 or 1)
// Response: status = 1 on success, -1 on failure.
void HandleSymbolSelect(long h, const uchar &payload[], uint len) {
    int off = 0;
    string sym = "";
    int enable = 0;
    if (!SafeUnpackStr(payload, off, len, sym) ||
        !SafeUnpackI32(payload, off, len, enable)) {
        SendError(h);
        return;
    }

    bool ok = SymbolSelect(sym, enable != 0);
    uchar empty[1];
    SendResponse(h, ok ? 1 : -1, empty, 0);
}

// ── Economic calendar handlers (protocol v9+) ────────────────────────────────

// Request layout (must match CalendarEvents in bridge_dll/mt5_bridge.cpp):
//   u64 from, u64 to (UTC seconds)
// Response: status = total event count, payload = packed records:
//   [u64 id][i64 time][u32 country_len][country][u32 currency_len][currency]
//   [u8 importance][u32 title_len][title][f64 actual][u8 actual_present]
//   [f64 forecast][u8 forecast_present][f64 previous][u8 previous_present]
// The stream is terminated by a record with id == 0.
void HandleCalendarEvents(long h, const uchar &payload[], uint len) {
    // CalendarValueHistory / MqlCalendarValue is not available in this MT5 build.
    // Full implementation preserved below for real terminals:
    /*
    int off = 0;
    ulong from = 0, to = 0;
    if (!SafeUnpackU64(payload, off, len, from) ||
        !SafeUnpackU64(payload, off, len, to)) {
        SendError(h);
        return;
    }

    string country_filter[];
    string currency_filter[];
    MqlCalendarValue values[];
    int total = CalendarValueHistory(country_filter, currency_filter,
                                     (datetime)from, (datetime)to, values);
    if (total < 0) {
        SendError(h);
        return;
    }

    uchar data[];
    for (int i = 0; i < total; i++) {
        PackU64(data, (ulong)values[i].id);
        PackI64(data, (long)values[i].time);
        PackStr(data, values[i].country);
        PackStr(data, values[i].currency);
        PackU8(data, (uchar)values[i].importance);
        PackStr(data, values[i].name);
        PackF64(data, values[i].actual);
        PackU8(data, values[i].actual != 0.0 ? 1 : 0);
        PackF64(data, values[i].forecast);
        PackU8(data, values[i].forecast != 0.0 ? 1 : 0);
        PackF64(data, values[i].previous);
        PackU8(data, values[i].previous != 0.0 ? 1 : 0);
    }
    // End-of-stream terminator: id == 0.
    PackU64(data, 0);
    SendResponse(h, total, data, (uint)ArraySize(data));
    */
    uchar empty[1];
    SendResponse(h, 0, empty, 0);
}

// Returns the countries that have calendar events. MT5 has no dedicated
// "enumerate calendar countries" call, so we scan a wide history window and
// de-duplicate the country field of every event.
// Response: status = country count, payload = [u32 len][bytes] per country,
// terminated by len == 0.
void HandleCalendarCountries(long h) {
    // CalendarValueHistory / MqlCalendarValue is not available in this MT5 build.
    // Full implementation preserved below for real terminals:
    /*
    datetime now = TimeCurrent();
    datetime from = now - 2 * 365 * 24 * 3600;
    datetime to = now + 2 * 365 * 24 * 3600;

    string country_filter[];
    string currency_filter[];
    MqlCalendarValue values[];
    int total = CalendarValueHistory(country_filter, currency_filter, from, to, values);
    if (total < 0) {
        SendError(h);
        return;
    }

    string countries[];
    for (int i = 0; i < total; i++) {
        string c = values[i].country;
        if (StringLen(c) == 0) continue;
        bool seen = false;
        for (int k = 0; k < ArraySize(countries); k++) {
            if (countries[k] == c) { seen = true; break; }
        }
        if (!seen) { int n=ArraySize(countries); ArrayResize(countries, n+1); countries[n]=c; }
    }

    uchar data[];
    for (int i = 0; i < ArraySize(countries); i++) {
        PackStr(data, countries[i]);
    }
    PackU32(data, 0); // end-of-stream terminator
    SendResponse(h, ArraySize(countries), data, (uint)ArraySize(data));
    */
    uchar empty[1];
    SendResponse(h, 0, empty, 0);
}

// Returns bridge-side statistics. The DLL tracks the actual counters
// (uptime, requests, drops, queue depth) and packs an Mt5Stats struct;
// this EA handler exists for protocol completeness and returns the
// negotiated protocol version as status.
void HandleStats(long h) {
    uchar data[];
    PackU32(data, PROTOCOL_VERSION);
    SendOkData(h, data, (uint)ArraySize(data));
}

// ── Request dispatcher ────────────────────────────────────────────────────────

bool DispatchRequest(long h) {
    // Read 16-byte request header: [uint32 length][uint8 kind][uint8 _pad][uint16 id][int32 status][uint32 request_id]
    uchar hdr[];
    if (!PipeReadExact(h, hdr, 16)) return false;

    uint pay_len = UnpackU32(hdr, 0);
    uchar kind   = hdr[4];
    uint cmd     = (uint)hdr[6] | ((uint)hdr[7] << 8);
    uint req_id  = UnpackU32(hdr, 12);

    g_current_cmd    = cmd;
    g_current_req_id = req_id;

    if (pay_len > MAX_PAYLOAD_SIZE) {
        Print("MT5Bridge: request payload length ", pay_len, " exceeds limit (", MAX_PAYLOAD_SIZE, ")");
        return false;
    }

    uchar payload[];
    if (pay_len > 0) {
        if (!PipeReadExact(h, payload, pay_len)) return false;
    }

    // Enforce authentication on all commands except CMD_INIT
    if (!g_authenticated && cmd != CMD_INIT) {
        Print("MT5Bridge: unauthenticated request rejected (cmd=", cmd, "). Client must authenticate via CMD_INIT.");
        SendError(h);
        return true;
    }

    switch (cmd) {
        case CMD_INIT:              HandleInit(h, payload, pay_len);              break;
        case CMD_SHUTDOWN:          HandleShutdown(h); return false;  /* disconnect */
        case CMD_RATES:             HandleCopyRates(h, payload, pay_len);         break;
        case CMD_ACCOUNT:           HandleAccount(h);                             break;
        case CMD_ORDER_SEND:        HandleOrderSend(h, payload, pay_len);         break;
        case CMD_ORDER_CLOSE:       HandleOrderClose(h, payload, pay_len);        break;
        case CMD_ORDER_MODIFY:      HandleOrderModify(h, payload, pay_len);       break;
        case CMD_SYM_TICK:          HandleSymbolInfoTick(h, payload, pay_len);    break;
        case CMD_SYM_INFO:          HandleSymbolInfoFull(h, payload, pay_len);    break;
        case CMD_POSITIONS_GET:     HandlePositionsGet(h, payload, pay_len);     break;
        case CMD_ORDERS_GET:        HandleOrdersGet(h, payload, pay_len);        break;
        case CMD_DEALS_GET:         HandleDealsGet(h, payload, pay_len);         break;
        case CMD_SUBSCRIBE_TICKS:   HandleSubscribeTicks(h, payload, pay_len);   break;
        case CMD_UNSUBSCRIBE_TICKS: HandleUnsubscribeTicks(h, payload, pay_len); break;
        case CMD_SUBSCRIBE_TRADE:   HandleSubscribeTrade(h);                     break;
        case CMD_UNSUBSCRIBE_TRADE: HandleUnsubscribeTrade(h);                   break;
        case CMD_SUBSCRIBE_BOOK:    HandleSubscribeBook(h, payload, pay_len);    break;
        case CMD_UNSUBSCRIBE_BOOK:  HandleUnsubscribeBook(h, payload, pay_len);  break;
        case CMD_SUBSCRIBE_BARS:    HandleSubscribeBars(h, payload, pay_len);    break;
        case CMD_UNSUBSCRIBE_BARS:  HandleUnsubscribeBars(h, payload, pay_len);  break;
        case CMD_PING:              HandlePing(h);                               break;
        case CMD_SYMBOLS_TOTAL:     HandleSymbolsTotal(h);                       break;
        case CMD_SYMBOLS_GET:       HandleSymbolsGet(h, payload, pay_len);       break;
        case CMD_SYMBOL_SELECT:     HandleSymbolSelect(h, payload, pay_len);     break;
        case CMD_CALENDAR_EVENTS:   HandleCalendarEvents(h, payload, pay_len);   break;
        case CMD_CALENDAR_COUNTRIES: HandleCalendarCountries(h);                break;
        case CMD_STATS:              HandleStats(h);                             break;
        default:
            Print("MT5Bridge: unknown cmd=", cmd);
            SendError(h);
            break;
    }
    return true;
}

// Helper to create the named pipe server with an explicit Win32 Security Descriptor.
// Fails closed if SDDL conversion is enabled but fails.
long CreatePipeHandle() {
    SECURITY_ATTRIBUTES sa;
    sa.nLength = sizeof(SECURITY_ATTRIBUTES);
    sa._padding = 0;
    sa.lpSecurityDescriptor = 0;
    sa.bInheritHandle = 0;
    sa._padding2 = 0;

    long pSD = 0;
    uint sdSize = 0;
    if (StringLen(InpPipeSDDL) > 0) {
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(InpPipeSDDL, 1 /* SDDL_REVISION_1 */, pSD, sdSize) || pSD == 0) {
            Print("MT5Bridge: FATAL — ConvertStringSecurityDescriptor failed; failing closed for security");
            return INVALID_HANDLE;
        }
        sa.lpSecurityDescriptor = pSD;
    }

    long h = CreateNamedPipeW(
        g_pipe_path,
        PIPE_ACCESS_DUPLEX | 0x00080000 /* FILE_FLAG_FIRST_PIPE_INSTANCE */,
        PIPE_TYPE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
        1,
        65536,
        65536,
        0,
        sa
    );

    if (pSD != 0) {
        LocalFree(pSD);
    }
    return h;
}

// Name of the takeover event, scoped to this pipe so multiple terminals/accounts
// using different pipe names do not interfere with each other.
string TakeoverEventName() {
    return "mt5bridge_takeover_" + InpPipeName;
}

// Terminal-wide record of the generation currently owning the pipe. MQL5 global
// variables are visible to every EA in the terminal, so a newer instance can
// compare its generation with the running one before requesting takeover.
// Global-variable names allow only [A-Za-z0-9_] and are limited to 63 chars, so
// sanitize and truncate the pipe name.
string GenerationVarName() {
    string out = "mt5bridge_gen_";
    int n = StringLen(InpPipeName);
    for (int i = 0; i < n && i < 40; i++) {
        ushort c = StringGetCharacter(InpPipeName, i);
        if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
            (c >= '0' && c <= '9') || c == '_')
            out += ShortToString(c);
        else
            out += "_";
    }
    return out;
}

// Close the pipe server/client handles exactly once. g_client aliases g_server
// while a client is connected, so only the alias is cleared before g_server.
void ReleasePipeServer() {
    g_authenticated = false;
    if (g_client != INVALID_HANDLE) {
        DisconnectNamedPipe(g_client);
        // Do not close g_client: g_client aliases g_server.
        g_client = INVALID_HANDLE;
    }
    if (g_server != INVALID_HANDLE) {
        CloseHandle(g_server);
        g_server = INVALID_HANDLE;
    }
}

// Close every chart except the one running this instance. Used by the headless
// setup so repeated restarts/takeovers do not accumulate stale charts. Charts
// are collected before closing because closing mutates the enumeration order.
void CloseStaleCharts() {
    long self = ChartID();
    long ids[];
    int n = 0;
    for (long ch = ChartFirst(); ch >= 0; ch = ChartNext(ch)) {
        if (ch == self) continue;
        ArrayResize(ids, n + 1);
        ids[n++] = ch;
    }
    for (int i = 0; i < n; i++) {
        Print("MT5Bridge: closing stale chart ", ids[i]);
        ChartClose(ids[i]);
    }
    g_stale_charts_closed = true;
    Print("MT5Bridge: stale chart cleanup complete (closed ", n, " chart(s))");
}

// ── EA lifecycle ──────────────────────────────────────────────────────────────

int OnInit() {
    if (InpRequireSecret && StringLen(InpPipeSecret) == 0) {
        Print("MT5Bridge: INIT_FAILED — InpRequireSecret is enabled but InpPipeSecret is empty! Set InpPipeSecret in EA inputs or set InpRequireSecret = false to opt out.");
        return INIT_FAILED;
    }

    if (InpServerTimeConvention < SERVER_TIME_FIXED ||
        InpServerTimeConvention > SERVER_TIME_EUROPE_DST) {
        Print("MT5Bridge: INIT_FAILED — invalid InpServerTimeConvention");
        return INIT_FAILED;
    }
    if (InpServerTimeConvention == SERVER_TIME_FIXED &&
        (InpServerUtcOffsetHours < -14 || InpServerUtcOffsetHours > 14)) {
        Print("MT5Bridge: INIT_FAILED — InpServerUtcOffsetHours must be between -14 and 14");
        return INIT_FAILED;
    }

    long account_type = AccountInfoInteger(ACCOUNT_TRADE_MODE);
    if (account_type == ACCOUNT_TRADE_MODE_REAL && InpRequireLiveAck && !InpLiveAckAcknowledged) {
        Print("MT5Bridge: INIT_FAILED — live trading account detected but InpLiveAckAcknowledged is false. Set InpLiveAckAcknowledged=true to permit live trading.");
        return INIT_FAILED;
    }

    long observed_offset = 0;
    if (TryObservedServerUtcOffset(observed_offset)) {
        long expected_offset = ExpectedServerUtcOffset(TimeTradeServer());
        Print("MT5Bridge: server-time observation convention=", ServerTimeConventionName(),
              " expected_offset=", expected_offset, "s observed_offset=", observed_offset,
              "s convert_to_utc=", (InpConvertToUTC ? "true" : "false"));
        if (InpConvertToUTC && observed_offset != expected_offset) {
            Print("MT5Bridge: WARNING — startup server-time offset differs from the configured convention");
        }
    } else {
        Print("MT5Bridge: WARNING — startup server-time offset could not be observed; UTC conversion will use the fallback guard");
    }

    g_pipe_path = "\\\\.\\pipe\\" + InpPipeName;
    g_takeover_event = CreateEventW(0, 1 /* manual reset */, 0, TakeoverEventName());
    if (g_takeover_event == 0)
        Print("MT5Bridge: WARNING — could not create takeover event; existing-instance refresh disabled");

    long my_gen = InpConfigGeneration;
    long prev_gen = (long)GlobalVariableGet(GenerationVarName());
    bool may_takeover = (my_gen > 0 && my_gen > prev_gen);

    g_takeover_start = GetTickCount();
    g_init_state = INIT_WAIT_TAKEOVER;

    // Try to bind the pipe once without blocking. If it is free, finish
    // initialization immediately; otherwise ask the owner to release it and let
    // the timer retry the bind so OnInit returns right away (non-blocking
    // takeover — OnInit never sleeps for the takeover timeout).
    if (BindPipe(my_gen)) {
        g_init_state = INIT_READY;
        return CompleteInitialization();
    }

    if (!may_takeover) {
        Print("MT5Bridge: CreateNamedPipe failed — another instance owns the pipe (generation ",
              prev_gen, "); this instance (generation ", my_gen,
              ") will not take over. Set InpConfigGeneration higher to refresh it.");
        return INIT_FAILED;
    }

    Print("MT5Bridge: existing instance detected (generation ", prev_gen,
          "); requesting takeover with generation ", my_gen);
    SetEvent(g_takeover_event);

    int timer_ms = (InpTimerIntervalMs > 0) ? InpTimerIntervalMs : 1;
    EventSetMillisecondTimer(timer_ms);
    Print("MT5Bridge: awaiting pipe release (timeout=", InpTakeoverTimeoutMs,
          "ms) — waiting for Rust client");
    return INIT_SUCCEEDED;
}

// Bind the named pipe once and, on success, record this instance's generation.
// Returns true when this instance now owns the pipe.
bool BindPipe(long my_gen) {
    g_server = CreatePipeHandle();
    if (g_server == INVALID_HANDLE) return false;
    // Record this instance's generation so a later start can decide whether it
    // is newer. Always write it (including 0) so a stale value cannot block a
    // legitimate refresh.
    GlobalVariableSet(GenerationVarName(), (double)my_gen);
    return true;
}

// Finish pipe-server initialization after the bind succeeds: mark the EA
// running, schedule optional stale-chart cleanup, and arm the timer.
int CompleteInitialization() {
    g_running = true;
    g_authenticated = false;
    // Give a concurrent takeover time to settle before removing other charts,
    // so we never close a chart whose EA is still releasing the pipe.
    if (InpCloseStaleCharts) {
        g_stale_cleanup_at = GetTickCount() + 3000;
        g_stale_charts_closed = false;
    }
    int timer_ms = (InpTimerIntervalMs > 0) ? InpTimerIntervalMs : 1;
    EventSetMillisecondTimer(timer_ms);
    Print("MT5Bridge: pipe server ready (timer=", timer_ms, "ms) — waiting for Rust client");
    return INIT_SUCCEEDED;
}

void OnDeinit(const int reason) {
    EventKillTimer();
    g_running = false;
    g_authenticated = false;

    // Release any book subscriptions
    int book_total = ArraySize(g_book_subscriptions);
    for (int i = 0; i < book_total; i++) {
        if (StringLen(g_book_subscriptions[i]) > 0) {
            MarketBookRelease(g_book_subscriptions[i]);
        }
    }
    ArrayResize(g_book_subscriptions, 0);
    ArrayResize(g_tick_subscriptions, 0);

    ReleasePipeServer();

    if (g_takeover_event != 0) {
        CloseHandle(g_takeover_event);
        g_takeover_event = 0;
    }
    Print("MT5Bridge: shutdown (reason=", reason, ")");
}

void ResetPipeServer(const string reason) {
    g_authenticated = false;
    if (g_server != INVALID_HANDLE) {
        DisconnectNamedPipe(g_server);
        g_client = INVALID_HANDLE;
        Print("MT5Bridge: reset pipe server (", reason, ")");
    } else {
        g_server = CreatePipeHandle();
        g_client = INVALID_HANDLE;
        if (g_server == INVALID_HANDLE)
            Print("MT5Bridge: failed to recreate pipe (", reason, ")");
        else
            Print("MT5Bridge: re-listening for next Rust client (", reason, ")");
    }
}

void OnTimer() {
    // A freshly started instance (normally the one launched from the headless
    // startup config) signals this event when it wants to take over. Release the
    // pipe immediately so it can bind, then unload this stale instance.
    if (g_takeover_event != 0 &&
        WaitForSingleObject(g_takeover_event, 0) == WAIT_OBJECT_0) {
        Print("MT5Bridge: takeover requested by a new instance — releasing pipe and refreshing");
        g_running = false;
        EventKillTimer();
        ReleasePipeServer();
        ExpertRemove();
        return;
    }

    // While waiting for the current owner to release the pipe, retry the bind
    // on each timer tick. OnInit never blocks for this (non-blocking takeover).
    if (g_init_state == INIT_WAIT_TAKEOVER) {
        uint elapsed = (uint)(GetTickCount() - g_takeover_start);
        uint timeout = (InpTakeoverTimeoutMs > 0) ? (uint)InpTakeoverTimeoutMs : 5000;
        if (BindPipe(InpConfigGeneration)) {
            g_init_state = INIT_READY;
            CompleteInitialization();
            Print("MT5Bridge: takeover complete");
        } else if (elapsed > timeout) {
            Print("MT5Bridge: takeover timed out after ", timeout,
                  "ms — existing instance did not release the pipe");
            if (g_takeover_event != 0) ResetEvent(g_takeover_event);
            ExpertRemove();
            return;
        }
    }

    if (!g_running || g_server == INVALID_HANDLE) return;

    // Once this instance owns the pipe, remove leftover charts from earlier
    // runs. The grace period lets a concurrent takeover finish first.
    if (InpCloseStaleCharts && !g_stale_charts_closed &&
        (int)(GetTickCount() - g_stale_cleanup_at) >= 0) {
        CloseStaleCharts();
    }

    // Poll for an incoming client connection without blocking the timer thread.
    if (g_client == INVALID_HANDLE) {
        ConnectNamedPipe(g_server, 0);
        uchar peek_conn[1];
        uint  peek_rd = 0, peek_av = 0, peek_lf = 0;
        if (PeekNamedPipe(g_server, peek_conn, 0, peek_rd, peek_av, peek_lf)) {
            g_client = g_server;
            uint mode = PIPE_READMODE_BYTE | PIPE_NOWAIT;
            SetNamedPipeHandleState(g_client, mode, 0, 0);
            g_authenticated = false;
            Print("MT5Bridge: Rust client connected (unauthenticated)");
        }
        return;
    }

    // Service a bounded number of requests per timer tick to prevent starving MT5's event loop.
    ulong start_us = GetMicrosecondCount();
    int processed = 0;

    while (processed < InpMaxRequestsPerTimer) {
        if (GetMicrosecondCount() - start_us >= InpMaxTimerBudgetUs) {
            break; // Time budget elapsed; yield to MT5 event loop until next timer tick
        }

        uchar peek_hdr[16];
        uint  peek_read = 0, peek_avail = 0, peek_left = 0;
        bool has_data = PeekNamedPipe(g_client, peek_hdr, 16,
                                      peek_read, peek_avail, peek_left);
        if (!has_data) {
            // Pipe broke or client disconnected abruptly without shutdown
            ResetPipeServer("client disconnected abruptly or pipe broken");
            return;
        }

        if (peek_avail < 16) break;  // need at least complete 16-byte header

        uint expected_pay_len = UnpackU32(peek_hdr, 0);
        if (expected_pay_len > MAX_PAYLOAD_SIZE) {
            Print("MT5Bridge: payload length ", expected_pay_len, " exceeds MAX_PAYLOAD_SIZE; dropping client");
            ResetPipeServer("exceeded MAX_PAYLOAD_SIZE");
            return;
        }

        // If full payload has not arrived yet, wait for next timer tick without blocking
        if (peek_avail < 16 + expected_pay_len) break;

        bool ok = DispatchRequest(g_client);
        if (!ok) {
            ResetPipeServer("client disconnected");
            return;
        }
        processed++;
    }

    // 2. Poll and enqueue ticks for subscribed non-chart symbols
    CheckSubscribedSymbols();

    // 3. Flush queued tick events to pipe
    FlushTickQueue();

    // 4. Flush queued trade events to pipe
    FlushTradeQueue();

    // 5. Poll subscribed bars on timer
    CheckSubscribedBars();
}

// Low-latency event-driven tick handler (protocol v5+ push model)
void OnTick() {
    if (!g_running || g_client == INVALID_HANDLE || !g_authenticated) return;

    string sym = _Symbol;
    int total = ArraySize(g_tick_subscriptions);
    for (int i = 0; i < total; i++) {
        if (g_tick_subscriptions[i].enabled && g_tick_subscriptions[i].symbol == sym) {
            MqlTick tick;
            if (SymbolInfoTick(sym, tick)) {
                if (CheckAndUpdateTick(i, tick)) {
                    EnqueueTick(sym, tick);
                }
            }
            break;
        }
    }

    CheckSubscribedBars();
    FlushTickQueue();
}

// Low-latency event-driven trade handler (protocol v6+ push model)
void OnTradeTransaction(const MqlTradeTransaction &trans,
                        const MqlTradeRequest &request,
                        const MqlTradeResult &result) {
    if (!g_running || g_client == INVALID_HANDLE || !g_authenticated || !g_trade_subscription)
        return;

    string cmt = request.comment;
    if (StringLen(cmt) == 0 && trans.deal > 0 && HistoryDealSelect(trans.deal)) {
        cmt = HistoryDealGetString(trans.deal, DEAL_COMMENT);
    }
    if (StringLen(cmt) == 0 && trans.order > 0 && HistoryOrderSelect(trans.order)) {
        cmt = HistoryOrderGetString(trans.order, ORDER_COMMENT);
    }

    int wire_trans_type = WTX_OTHER;
    switch (trans.type) {
        case TRADE_TRANSACTION_DEAL_ADD:     wire_trans_type = WTX_DEAL_ADD;     break;
        case TRADE_TRANSACTION_ORDER_ADD:    wire_trans_type = WTX_ORDER_ADD;    break;
        case TRADE_TRANSACTION_ORDER_DELETE: wire_trans_type = WTX_ORDER_DELETE; break;
        case TRADE_TRANSACTION_ORDER_UPDATE: wire_trans_type = WTX_ORDER_UPDATE; break;
        case TRADE_TRANSACTION_POSITION:     wire_trans_type = WTX_POSITION;     break;
        case TRADE_TRANSACTION_REQUEST:      wire_trans_type = WTX_REQUEST;      break;
        default:                             wire_trans_type = WTX_OTHER;        break;
    }

    int deal_entry = 0;
    int deal_type = 0;
    ulong deal_magic = 0;
    ulong deal_position_id = 0;
    int deal_reason = 0;

    if (trans.deal > 0 && HistoryDealSelect(trans.deal)) {
        deal_entry = (int)HistoryDealGetInteger(trans.deal, DEAL_ENTRY);
        deal_type = (int)HistoryDealGetInteger(trans.deal, DEAL_TYPE);
        deal_magic = (ulong)HistoryDealGetInteger(trans.deal, DEAL_MAGIC);
        deal_position_id = (ulong)HistoryDealGetInteger(trans.deal, DEAL_POSITION_ID);
        deal_reason = (int)HistoryDealGetInteger(trans.deal, DEAL_REASON);
    }
    if (deal_magic == 0 && request.magic > 0) {
        deal_magic = request.magic;
    }
    if (deal_position_id == 0 && trans.position > 0) {
        deal_position_id = trans.position;
    }

    long trade_time = InpConvertToUTC ? (long)TimeGMT() : (long)TimeCurrent();
    EnqueueTrade(trans.deal, trans.order, trans.position, trade_time,
                 wire_trans_type, (int)trans.order_type, trans.price,
                 trans.volume, trans.price_sl, trans.price_tp,
                 trans.symbol, cmt,
                 deal_entry, deal_type, deal_magic, deal_position_id, deal_reason);
    FlushTradeQueue();
}

// Low-latency event-driven depth-of-market handler (protocol v6+ push model)
void OnBookEvent(const string &symbol) {
    if (!g_running || g_client == INVALID_HANDLE || !g_authenticated)
        return;

    // Check if symbol is in book subscriptions
    int total = ArraySize(g_book_subscriptions);
    bool subscribed = false;
    for (int i = 0; i < total; i++) {
        if (g_book_subscriptions[i] == symbol) { subscribed = true; break; }
    }
    if (!subscribed) return;

    MqlBookInfo book[];
    if (MarketBookGet(symbol, book)) {
        int book_count = ArraySize(book);
        long now_msc = ServerTimeMsc();
        for (int i = 0; i < book_count; i++) {
            uchar payload[];
            PackFixedString32(payload, symbol);
            PackI64(payload, now_msc);
            PackI32(payload, (book[i].type == BOOK_TYPE_BUY) ? 1 : 2);
            PackI32(payload, 0); // padding
            PackF64(payload, book[i].price);
            PackF64(payload, (double)book[i].volume);
            PackU64(payload, ++g_event_seq);
            SendEvent(g_client, EVENT_BOOK, payload, 72);
        }
        if (book_count > 0) {
            FlushFileBuffers(g_client);
        }
    }
}

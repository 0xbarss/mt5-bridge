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

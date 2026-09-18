/* mt5_bridge.cpp
 *
 * Named-pipe CLIENT loaded by Rust via libloading.
 * Each exported function serialises its arguments into a binary packet,
 * sends it over \\.\pipe\mt5bridge, and deserialises the response.
 *
 * The EA (mt5_bridge.mq5) is the pipe SERVER — start it first.
 *
 * Build (MinGW-w64):
 *   x86_64-w64-mingw32-g++ -O2 -shared -o mt5_bridge.dll mt5_bridge.cpp
 *       -DMT5_BRIDGE_EXPORTS -std=c++17 -lkernel32
 *
 * Build (MSVC):
 *   cl /O2 /LD /DMT5_BRIDGE_EXPORTS mt5_bridge.cpp /link kernel32.lib
 */

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <cstring>
#include <cstdint>
#include <cstdlib>
#include <string>
#include "mt5_bridge.h"

/* ── Protocol ────────────────────────────────────────────────────────────── */
/*
 * Request  : [uint32 cmd][uint32 payload_len][payload...]
 * Response : [int32  status][uint32 data_len][data...]
 *
 * status >= 0  → success (for CopyRates == bar count; others == 1)
 * status <  0  → error
 *
 * Strings in payloads: [uint32 len][len bytes ASCII, no NUL]
 */

enum Cmd : uint32_t {
    CMD_INIT         = 1,
    CMD_SHUTDOWN     = 2,
    CMD_RATES        = 3,
    CMD_ACCOUNT      = 4,
    CMD_ORDER_SEND   = 5,
    CMD_ORDER_CLOSE  = 6,
    CMD_ORDER_MODIFY = 7,
    CMD_SYM_TICK     = 8,
    CMD_SYM_INFO     = 9,
};

/* ── Pipe state ───────────────────────────────────────────────────────────── */

static HANDLE           g_pipe = INVALID_HANDLE_VALUE;
static CRITICAL_SECTION g_cs;

/* RAII guard for g_cs. */
struct Lock { Lock() { EnterCriticalSection(&g_cs); } ~Lock() { LeaveCriticalSection(&g_cs); } };

BOOL WINAPI DllMain(HINSTANCE, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) InitializeCriticalSection(&g_cs);
    if (reason == DLL_PROCESS_DETACH) DeleteCriticalSection(&g_cs);
    return TRUE;
}

/* ── Low-level I/O ───────────────────────────────────────────────────────── */

static bool write_all(const void* data, DWORD len) {
    const char* p = static_cast<const char*>(data);
    DWORD done = 0;
    while (done < len) {
        DWORD n = 0;
        if (!WriteFile(g_pipe, p + done, len - done, &n, nullptr) || n == 0)
            return false;
        done += n;
    }
    return true;
}

static bool read_all(void* data, DWORD len) {
    char* p = static_cast<char*>(data);
    DWORD done = 0;
    while (done < len) {
        DWORD n = 0;
        if (!ReadFile(g_pipe, p + done, len - done, &n, nullptr) || n == 0)
            return false;
        done += n;
    }
    return true;
}

/* ── Packet helpers ──────────────────────────────────────────────────────── */

struct ReqHdr  { uint32_t cmd; uint32_t len; };
struct RespHdr { int32_t  status; uint32_t len; };

static bool send_packet(Cmd cmd, const std::string& payload) {
    ReqHdr hdr{ static_cast<uint32_t>(cmd),
                static_cast<uint32_t>(payload.size()) };
    return write_all(&hdr, sizeof hdr) &&
           (payload.empty() || write_all(payload.data(),
                                         static_cast<DWORD>(payload.size())));
}

static bool recv_packet(int32_t& status, std::string& data) {
    RespHdr hdr{};
    if (!read_all(&hdr, sizeof hdr)) return false;
    status = hdr.status;
    data.resize(hdr.len);
    return hdr.len == 0 || read_all(&data[0], hdr.len);
}

/* ── Payload builder ─────────────────────────────────────────────────────── */

struct Packer {
    std::string buf;

    void i32(int32_t  v) { append(&v, 4); }
    void i64(int64_t  v) { append(&v, 8); }
    void u64(uint64_t v) { append(&v, 8); }
    void f64(double   v) { append(&v, 8); }
    void str(const char* s) {
        uint32_t n = static_cast<uint32_t>(std::strlen(s));
        append(&n, 4);
        buf.append(s, n);
    }

private:
    void append(const void* p, size_t n) {
        buf.append(static_cast<const char*>(p), n);
    }
};

/* ── Exported functions ──────────────────────────────────────────────────── */

extern "C" {

int Initialize(long login, const char* password, const char* server) {
    Lock lk;

    if (g_pipe != INVALID_HANDLE_VALUE) {
        CloseHandle(g_pipe);
        g_pipe = INVALID_HANDLE_VALUE;
    }

    // Determine pipe name from environment or fallback to default
    char pipe_name[256] = {0};
    DWORD env_len = GetEnvironmentVariableA("MT5_PIPE_NAME", pipe_name, sizeof(pipe_name));
    std::string pipe_path = "\\\\.\\pipe\\";
    if (env_len > 0 && env_len < sizeof(pipe_name)) {
        pipe_path += pipe_name;
    } else {
        pipe_path += "mt5bridge";
    }

    /* Retry for 30 s while EA is starting. */
    HANDLE pipe = INVALID_HANDLE_VALUE;
    for (int i = 0; i < 60 && pipe == INVALID_HANDLE_VALUE; ++i) {
        pipe = CreateFileA(pipe_path.c_str(),
                           GENERIC_READ | GENERIC_WRITE,
                           0, nullptr, OPEN_EXISTING, 0, nullptr);
        if (pipe == INVALID_HANDLE_VALUE) {
            DWORD err = GetLastError();
            if (err == ERROR_PIPE_BUSY)
                WaitNamedPipeA(pipe_path.c_str(), 500);
            else
                Sleep(500);
        }
    }
    if (pipe == INVALID_HANDLE_VALUE) return 0;

    /* Switch to byte-stream mode. */
    DWORD mode = PIPE_READMODE_BYTE;
    SetNamedPipeHandleState(pipe, &mode, nullptr, nullptr);
    g_pipe = pipe;

    Packer p;
    p.i64(static_cast<int64_t>(login));
    p.str(password);
    p.str(server);

    if (!send_packet(CMD_INIT, p.buf)) return 0;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int Shutdown(void) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return 1;
    send_packet(CMD_SHUTDOWN, {});
    CloseHandle(g_pipe);
    g_pipe = INVALID_HANDLE_VALUE;
    return 1;
}

int CopyRates(const char* symbol, int timeframe, int64_t from,
              int64_t to, Mt5Rate* buf) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return -1;

    Packer p;
    p.str(symbol);
    p.i32(timeframe);
    p.i64(from);
    p.i64(to);

    if (!send_packet(CMD_RATES, p.buf)) return -1;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data) || st < 0) return -1;

    int32_t filled = st;
    int32_t max_n  = (int32_t)(data.size() / sizeof(Mt5Rate));
    int32_t n      = (filled < max_n) ? filled : max_n;
    if (n > 0)
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Rate));
    return n;
}

int AccountInfo(double* balance, double* equity,
                double* margin,  double* free_margin) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return 0;

    if (!send_packet(CMD_ACCOUNT, {})) return 0;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data) || st != 1 || data.size() < 32) return 0;

    std::memcpy(balance,      data.data(),      8);
    std::memcpy(equity,       data.data() +  8, 8);
    std::memcpy(margin,       data.data() + 16, 8);
    std::memcpy(free_margin,  data.data() + 24, 8);
    return 1;
}

int OrderSend(const char* symbol, int type, double volume,
              double price, double sl, double tp,
              const char* comment, Mt5TradeResult* result) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return 0;

    Packer p;
    p.str(symbol);
    p.i32(type);
    p.f64(volume);
    p.f64(price);
    p.f64(sl);
    p.f64(tp);
    p.str(comment);

    if (!send_packet(CMD_ORDER_SEND, p.buf)) return 0;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data) || st != 1 || data.size() < sizeof(Mt5TradeResult))
        return 0;

    std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
    return 1;
}

int OrderClose(uint64_t ticket, Mt5TradeResult* result) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return 0;

    Packer p;
    p.u64(ticket);

    if (!send_packet(CMD_ORDER_CLOSE, p.buf)) return 0;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data) || st != 1 || data.size() < sizeof(Mt5TradeResult))
        return 0;

    std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
    return 1;
}

int OrderModify(uint64_t ticket, double sl, double tp) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return 0;

    Packer p;
    p.u64(ticket);
    p.f64(sl);
    p.f64(tp);

    if (!send_packet(CMD_ORDER_MODIFY, p.buf)) return 0;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int SymbolInfoFull(const char* symbol, Mt5SymInfo* info) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return 0;

    Packer p;
    p.str(symbol);

    if (!send_packet(CMD_SYM_INFO, p.buf)) return 0;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data) || st != 1 || data.size() < sizeof(Mt5SymInfo))
        return 0;

    std::memcpy(info, data.data(), sizeof(Mt5SymInfo));
    return 1;
}

int SymbolInfoTick(const char* symbol, Mt5Tick* tick) {
    Lock lk;
    if (g_pipe == INVALID_HANDLE_VALUE) return 0;

    Packer p;
    p.str(symbol);

    if (!send_packet(CMD_SYM_TICK, p.buf)) return 0;

    int32_t st = 0; std::string data;
    if (!recv_packet(st, data) || st != 1 || data.size() < sizeof(Mt5Tick))
        return 0;

    std::memcpy(tick, data.data(), sizeof(Mt5Tick));
    return 1;
}

} /* extern "C" */

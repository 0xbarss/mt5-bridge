/* mt5_bridge.cpp
 *
 * Named-pipe CLIENT loaded by Rust via libloading.
 * Implements protocol v5 full-duplex client with dedicated background reader thread,
 * asynchronous event dispatching, and synchronous request/response multiplexing.
 *
 * Build (MinGW-w64):
 *   x86_64-w64-mingw32-g++ -O2 -shared -o mt5_bridge.dll mt5_bridge.cpp
 *       -DMT5_BRIDGE_EXPORTS -std=c++17 -lkernel32
 */

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <cstring>
#include <cstdint>
#include <cstdlib>
#include <string>
#include <vector>
#include <queue>
#include <thread>
#include <mutex>
#include <condition_variable>
#include <atomic>
#include <chrono>
#include "mt5_bridge.h"

/* ── Protocol ────────────────────────────────────────────────────────────── */
/*
 * Protocol v5: Unified 12-byte packet header:
 *   [uint32 length][uint8 kind][uint8 _pad][uint16 id][int32 status]
 *
 * kind:
 *   0 = PKT_REQUEST  : id = Cmd, status = 0, length = payload_size
 *   1 = PKT_RESPONSE : id = Cmd (matching request), status >= 0 (success/count) or < 0 (err)
 *   2 = PKT_EVENT    : id = EventKind (TICK/TRADE/BOOK), status = 0, length = payload_size
 *
 * Strings in payloads: [uint32 len][len bytes ASCII, no NUL]
 */

enum Cmd : uint32_t {
    CMD_INIT              = 1,
    CMD_SHUTDOWN          = 2,
    CMD_RATES             = 3,
    CMD_ACCOUNT           = 4,
    CMD_ORDER_SEND        = 5,
    CMD_ORDER_CLOSE       = 6,
    CMD_ORDER_MODIFY      = 7,
    CMD_SYM_TICK          = 8,
    CMD_SYM_INFO          = 9,
    CMD_POSITIONS_GET     = 10,
    CMD_ORDERS_GET        = 11,
    CMD_DEALS_GET         = 12,
    CMD_SUBSCRIBE_TICKS   = 13,
    CMD_UNSUBSCRIBE_TICKS = 14,
    CMD_SUBSCRIBE_TRADE   = 15,
    CMD_UNSUBSCRIBE_TRADE = 16,
    CMD_SUBSCRIBE_BOOK    = 17,
    CMD_UNSUBSCRIBE_BOOK  = 18,
};

enum PacketKind : uint8_t {
    PKT_REQUEST  = 0,
    PKT_RESPONSE = 1,
    PKT_EVENT    = 2,
};

enum EventKind : uint16_t {
    EVENT_TICK  = 1,
    EVENT_TRADE = 2,
    EVENT_BOOK  = 3,
    EVENT_BAR   = 4,
};

/* ── State ────────────────────────────────────────────────────────────────── */

static HANDLE                   g_pipe = INVALID_HANDLE_VALUE;
static std::mutex               g_write_mutex;

static std::mutex               g_request_mutex;
static std::condition_variable  g_request_cv;
static bool                     g_request_pending = false;
static uint16_t                 g_pending_cmd     = 0;
static bool                     g_pending_done    = false;
static int32_t                  g_pending_status  = 0;
static std::string              g_pending_data;

static std::thread              g_reader_thread;
static std::atomic<bool>        g_running{false};

static Mt5EventCallback         g_event_callback = nullptr;
static std::mutex               g_event_mutex;
static std::condition_variable  g_event_cv;

struct QueuedRawEvent {
    uint16_t    event_type;
    std::string data;
};
static const size_t             MAX_EVENT_QUEUE = 8192;
static std::queue<QueuedRawEvent> g_event_queue;

/* ── Low-level I/O ───────────────────────────────────────────────────────── */

static bool write_all_raw(const void* data, DWORD len) {
    if (g_pipe == INVALID_HANDLE_VALUE) return false;
    const char* p = static_cast<const char*>(data);
    DWORD done = 0;
    while (done < len) {
        DWORD n = 0;
        if (!WriteFile(g_pipe, p + done, len - done, &n, nullptr) || n == 0) {
            return false;
        }
        done += n;
    }
    return true;
}

static bool read_all_raw(void* data, DWORD len) {
    if (g_pipe == INVALID_HANDLE_VALUE) return false;
    char* p = static_cast<char*>(data);
    DWORD done = 0;
    while (done < len) {
        DWORD n = 0;
        if (!ReadFile(g_pipe, p + done, len - done, &n, nullptr) || n == 0) {
            return false;
        }
        done += n;
    }
    return true;
}

/* ── Reader loop ─────────────────────────────────────────────────────────── */

static void reader_loop() {
    const uint32_t MAX_PAYLOAD = 16 * 1024 * 1024; // 16 MB

    while (g_running.load()) {
        PacketHdr hdr{};
        if (!read_all_raw(&hdr, sizeof(hdr))) {
            break;
        }

        if (hdr.length > MAX_PAYLOAD) {
            break;
        }

        std::string payload;
        if (hdr.length > 0) {
            payload.resize(hdr.length);
            if (!read_all_raw(&payload[0], hdr.length)) {
                break;
            }
        }

        if (hdr.kind == PKT_RESPONSE) {
            std::lock_guard<std::mutex> lk(g_request_mutex);
            if (g_request_pending && g_pending_cmd == hdr.id) {
                g_pending_status = hdr.status;
                g_pending_data   = std::move(payload);
                g_pending_done   = true;
                g_request_cv.notify_one();
            }
        } else if (hdr.kind == PKT_EVENT) {
            Mt5EventCallback cb = nullptr;
            {
                std::lock_guard<std::mutex> lk(g_event_mutex);
                cb = g_event_callback;
                if (g_event_queue.size() >= MAX_EVENT_QUEUE) {
                    g_event_queue.pop(); // drop oldest to preserve real-time responsiveness
                }
                g_event_queue.push({hdr.id, payload});
                g_event_cv.notify_one();
            }
            if (cb) {
                cb(hdr.id, payload.data(), static_cast<uint32_t>(payload.size()));
            }
        }
    }

    // Wake up any waiting request if pipe severed
    {
        std::lock_guard<std::mutex> lk(g_request_mutex);
        if (g_request_pending && !g_pending_done) {
            g_pending_status = MT5_ERR_PIPE_DISCONNECTED;
            g_pending_done   = true;
            g_request_cv.notify_one();
        }
    }
    g_event_cv.notify_all();
}

/* ── Request execution ───────────────────────────────────────────────────── */

static bool send_request_and_wait(Cmd cmd, const std::string& payload,
                                  int32_t& out_status, std::string& out_data,
                                  int timeout_sec = 20) {
    if (g_pipe == INVALID_HANDLE_VALUE || !g_running.load()) {
        out_status = MT5_ERR_PIPE_DISCONNECTED;
        return false;
    }

    std::unique_lock<std::mutex> req_lk(g_request_mutex);
    g_request_pending = true;
    g_pending_cmd     = static_cast<uint16_t>(cmd);
    g_pending_done    = false;
    g_pending_status  = 0;
    g_pending_data.clear();

    PacketHdr hdr{};
    hdr.length = static_cast<uint32_t>(payload.size());
    hdr.kind   = PKT_REQUEST;
    hdr._pad   = 0;
    hdr.id     = static_cast<uint16_t>(cmd);
    hdr.status = 0;

    {
        std::lock_guard<std::mutex> write_lk(g_write_mutex);
        if (!write_all_raw(&hdr, sizeof(hdr)) ||
            (!payload.empty() && !write_all_raw(payload.data(), static_cast<DWORD>(payload.size())))) {
            g_request_pending = false;
            out_status = MT5_ERR_SEND_FAILED;
            return false;
        }
    }

    bool ok = g_request_cv.wait_for(req_lk, std::chrono::seconds(timeout_sec), [&]() {
        return g_pending_done || !g_running.load();
    });

    g_request_pending = false;

    if (!ok || !g_pending_done) {
        out_status = MT5_ERR_UNKNOWN_EXECUTION;
        return false;
    }

    out_status = g_pending_status;
    out_data   = std::move(g_pending_data);
    return true;
}

static void stop_reader_and_disconnect() {
    g_running.store(false);

    if (g_pipe != INVALID_HANDLE_VALUE) {
        HANDLE h = g_pipe;
        g_pipe = INVALID_HANDLE_VALUE;
        CloseHandle(h);
    }

    if (g_reader_thread.joinable()) {
        g_reader_thread.join();
    }

    {
        std::lock_guard<std::mutex> lk(g_request_mutex);
        g_pending_done = true;
        g_request_cv.notify_all();
    }
    g_event_cv.notify_all();
}

/* ── Payload builder ─────────────────────────────────────────────────────── */

struct Packer {
    std::string buf;

    void u32(uint32_t v) { append(&v, 4); }
    void i32(int32_t  v) { append(&v, 4); }
    void i64(int64_t  v) { append(&v, 8); }
    void u64(uint64_t v) { append(&v, 8); }
    void f64(double   v) { append(&v, 8); }
    void str(const char* s) {
        if (!s) s = "";
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

int Initialize(int64_t login, const char* password, const char* server) {
    stop_reader_and_disconnect();

    char pipe_name[256] = {0};
    DWORD env_len = GetEnvironmentVariableA("MT5_PIPE_NAME", pipe_name, sizeof(pipe_name));
    std::string pipe_path = "\\\\.\\pipe\\";
    if (env_len > 0 && env_len < sizeof(pipe_name)) {
        pipe_path += pipe_name;
    } else {
        pipe_path += "mt5bridge";
    }

    HANDLE pipe = INVALID_HANDLE_VALUE;
    for (int i = 0; i < 60 && pipe == INVALID_HANDLE_VALUE; ++i) {
        pipe = CreateFileA(pipe_path.c_str(),
                           GENERIC_READ | GENERIC_WRITE,
                           0, nullptr, OPEN_EXISTING,
                           SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, nullptr);
        if (pipe == INVALID_HANDLE_VALUE) {
            DWORD err = GetLastError();
            if (err == ERROR_PIPE_BUSY)
                WaitNamedPipeA(pipe_path.c_str(), 500);
            else
                Sleep(500);
        }
    }
    if (pipe == INVALID_HANDLE_VALUE) return 0;

    DWORD mode = PIPE_READMODE_BYTE;
    SetNamedPipeHandleState(pipe, &mode, nullptr, nullptr);

    ULONG srv_pid = 0;
    typedef BOOL (WINAPI *FnGetNamedPipeServerProcessId)(HANDLE, PULONG);
    HMODULE k32 = GetModuleHandleA("kernel32.dll");
    if (k32) {
        FnGetNamedPipeServerProcessId fn_get_pid =
            (FnGetNamedPipeServerProcessId)GetProcAddress(k32, "GetNamedPipeServerProcessId");
        if (fn_get_pid && fn_get_pid(pipe, &srv_pid)) {
            if (srv_pid != 0 && srv_pid == GetCurrentProcessId()) {
                CloseHandle(pipe);
                return 0;
            }
        }
    }

    g_pipe = pipe;
    g_running.store(true);

    // Clear event queue on new connection
    {
        std::lock_guard<std::mutex> lk(g_event_mutex);
        while (!g_event_queue.empty()) g_event_queue.pop();
    }

    // Launch background pipe reader thread
    g_reader_thread = std::thread(reader_loop);

    std::string auth_token = (password != nullptr) ? password : "";
    if (auth_token.empty()) {
        char secret_buf[256] = {0};
        DWORD sec_len = GetEnvironmentVariableA("MT5_PIPE_SECRET", secret_buf, sizeof(secret_buf));
        if (sec_len > 0 && sec_len < sizeof(secret_buf)) {
            auth_token = secret_buf;
        }
    }

    Packer p;
    p.i64(static_cast<int64_t>(login));
    p.str(auth_token.c_str());
    p.str(server ? server : "");
    p.u32(MT5_BRIDGE_PROTOCOL_VERSION);

    int32_t st = 0;
    std::string data;
    if (!send_request_and_wait(CMD_INIT, p.buf, st, data)) {
        stop_reader_and_disconnect();
        return 0;
    }

    if (st != 1) {
        stop_reader_and_disconnect();
        return 0;
    }

    return 1;
}

int Shutdown(void) {
    if (g_pipe != INVALID_HANDLE_VALUE && g_running.load()) {
        int32_t st = 0; std::string data;
        send_request_and_wait(CMD_SHUTDOWN, {}, st, data, 2);
    }
    stop_reader_and_disconnect();
    return 1;
}

int CopyRates(const char* symbol, int timeframe, int64_t from,
              int64_t to, Mt5Rate* buf, int buf_capacity) {
    if (!buf || buf_capacity <= 0) return -1;

    Packer p;
    p.str(symbol);
    p.i32(timeframe);
    p.i64(from);
    p.i64(to);
    p.i32(buf_capacity);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_RATES, p.buf, st, data) || st < 0) return -1;

    int32_t filled = st;
    int32_t max_n  = (int32_t)(data.size() / sizeof(Mt5Rate));
    int32_t n      = (filled < max_n) ? filled : max_n;
    if (n > buf_capacity) n = buf_capacity;
    if (n > 0)
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Rate));
    return n;
}

int AccountInfo(double* balance, double* equity,
                double* margin,  double* free_margin) {
    if (!balance || !equity || !margin || !free_margin) return 0;

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_ACCOUNT, {}, st, data) || st != 1 || data.size() < 32) return 0;

    std::memcpy(balance,      data.data(),      8);
    std::memcpy(equity,       data.data() +  8, 8);
    std::memcpy(margin,       data.data() + 16, 8);
    std::memcpy(free_margin,  data.data() + 24, 8);
    return 1;
}

int OrderSend(const char* symbol, int type, double volume,
              double price, double sl, double tp,
              const char* comment, uint32_t deviation,
              int64_t expiration, uint64_t magic,
              Mt5TradeResult* result) {
    Packer p;
    p.str(symbol);
    p.i32(type);
    p.f64(volume);
    p.f64(price);
    p.f64(sl);
    p.f64(tp);
    p.str(comment);
    p.u32(deviation);
    p.i64(expiration);
    p.u64(magic);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_ORDER_SEND, p.buf, st, data)) {
        return (st == MT5_ERR_PIPE_DISCONNECTED) ? MT5_ERR_PIPE_DISCONNECTED :
               (st == MT5_ERR_SEND_FAILED) ? MT5_ERR_SEND_FAILED : MT5_ERR_UNKNOWN_EXECUTION;
    }

    if (result && data.size() >= sizeof(Mt5TradeResult)) {
        std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
    }
    return (st == 1) ? MT5_OK : MT5_ERR_GENERAL;
}

int OrderCloseWithMagic(uint64_t ticket, uint64_t magic, Mt5TradeResult* result) {
    Packer p;
    p.u64(ticket);
    p.u64(magic);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_ORDER_CLOSE, p.buf, st, data)) {
        return (st == MT5_ERR_PIPE_DISCONNECTED) ? MT5_ERR_PIPE_DISCONNECTED :
               (st == MT5_ERR_SEND_FAILED) ? MT5_ERR_SEND_FAILED : MT5_ERR_UNKNOWN_EXECUTION;
    }

    if (result && data.size() >= sizeof(Mt5TradeResult)) {
        std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
    }
    return (st == 1) ? MT5_OK : MT5_ERR_GENERAL;
}

int OrderClose(uint64_t ticket, Mt5TradeResult* result) {
    return OrderCloseWithMagic(ticket, 0, result);
}

int OrderModifyWithMagic(uint64_t ticket, uint64_t magic, double sl, double tp, Mt5TradeResult* result) {
    Packer p;
    p.u64(ticket);
    p.f64(sl);
    p.f64(tp);
    p.u64(magic);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_ORDER_MODIFY, p.buf, st, data)) {
        return (st == MT5_ERR_PIPE_DISCONNECTED) ? MT5_ERR_PIPE_DISCONNECTED :
               (st == MT5_ERR_SEND_FAILED) ? MT5_ERR_SEND_FAILED : MT5_ERR_UNKNOWN_EXECUTION;
    }

    if (result && data.size() >= sizeof(Mt5TradeResult)) {
        std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
    }
    return (st == 1) ? MT5_OK : MT5_ERR_GENERAL;
}

int OrderModify(uint64_t ticket, double sl, double tp, Mt5TradeResult* result) {
    return OrderModifyWithMagic(ticket, 0, sl, tp, result);
}

int PositionsGet(Mt5Position* buf, int buf_capacity, uint64_t magic_filter, const char* symbol_filter) {
    if (!buf || buf_capacity <= 0) return -1;

    Packer p;
    p.u64(magic_filter);
    p.str(symbol_filter ? symbol_filter : "");
    p.i32(buf_capacity);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_POSITIONS_GET, p.buf, st, data) || st < 0) return -1;

    int32_t count = st;
    int32_t max_items = static_cast<int32_t>(data.size() / sizeof(Mt5Position));
    int32_t n = (count < max_items) ? count : max_items;
    if (n > buf_capacity) n = buf_capacity;
    if (n > 0) {
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Position));
    }
    return n;
}

int OrdersGet(Mt5Order* buf, int buf_capacity, uint64_t magic_filter, const char* symbol_filter) {
    if (!buf || buf_capacity <= 0) return -1;

    Packer p;
    p.u64(magic_filter);
    p.str(symbol_filter ? symbol_filter : "");
    p.i32(buf_capacity);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_ORDERS_GET, p.buf, st, data) || st < 0) return -1;

    int32_t count = st;
    int32_t max_items = static_cast<int32_t>(data.size() / sizeof(Mt5Order));
    int32_t n = (count < max_items) ? count : max_items;
    if (n > buf_capacity) n = buf_capacity;
    if (n > 0) {
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Order));
    }
    return n;
}

int DealsGet(Mt5Deal* buf, int buf_capacity, int64_t from, int64_t to, uint64_t magic_filter, const char* symbol_filter) {
    if (!buf || buf_capacity <= 0) return -1;

    /* Request layout (must match HandleDealsGet in the EA): from, to, magic, symbol, capacity */
    Packer p;
    p.i64(from);
    p.i64(to);
    p.u64(magic_filter);
    p.str(symbol_filter ? symbol_filter : "");
    p.i32(buf_capacity);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_DEALS_GET, p.buf, st, data) || st < 0) return -1;

    int32_t count = st;
    int32_t max_items = static_cast<int32_t>(data.size() / sizeof(Mt5Deal));
    int32_t n = (count < max_items) ? count : max_items;
    if (n > buf_capacity) n = buf_capacity;
    if (n > 0) {
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Deal));
    }
    return n;
}

int SymbolInfoFull(const char* symbol, Mt5SymInfo* info) {
    if (!info) return 0;

    Packer p;
    p.str(symbol);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_SYM_INFO, p.buf, st, data) || st != 1 || data.size() < sizeof(Mt5SymInfo))
        return 0;

    std::memcpy(info, data.data(), sizeof(Mt5SymInfo));
    return 1;
}

int SymbolInfoTick(const char* symbol, Mt5Tick* tick) {
    if (!tick) return 0;

    Packer p;
    p.str(symbol);

    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_SYM_TICK, p.buf, st, data) || st != 1 || data.size() < sizeof(Mt5Tick))
        return 0;

    std::memcpy(tick, data.data(), sizeof(Mt5Tick));
    return 1;
}

/* ── Subscription Management (Protocol v5+) ──────────────────────────────── */

int SubscribeTicks(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    Packer p;
    p.str(symbol);
    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_SUBSCRIBE_TICKS, p.buf, st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int UnsubscribeTicks(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    Packer p;
    p.str(symbol);
    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_UNSUBSCRIBE_TICKS, p.buf, st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int SubscribeTrade(void) {
    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_SUBSCRIBE_TRADE, {}, st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int UnsubscribeTrade(void) {
    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_UNSUBSCRIBE_TRADE, {}, st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int SubscribeBook(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    Packer p;
    p.str(symbol);
    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_SUBSCRIBE_BOOK, p.buf, st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int UnsubscribeBook(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    Packer p;
    p.str(symbol);
    int32_t st = 0; std::string data;
    if (!send_request_and_wait(CMD_UNSUBSCRIBE_BOOK, p.buf, st, data)) return 0;
    return (st == 1) ? 1 : 0;
}

int RegisterEventCallback(Mt5EventCallback cb) {
    std::lock_guard<std::mutex> lk(g_event_mutex);
    g_event_callback = cb;
    return 1;
}

int PollEvent(uint16_t* out_event_type, void* out_buf, uint32_t buf_cap, uint32_t* out_len, uint32_t timeout_ms) {
    if (!out_event_type || !out_buf || buf_cap == 0) return 0;

    std::unique_lock<std::mutex> lk(g_event_mutex);
    if (g_event_queue.empty()) {
        if (timeout_ms == 0) return 0;
        bool got = g_event_cv.wait_for(lk, std::chrono::milliseconds(timeout_ms), []() {
            return !g_event_queue.empty() || !g_running.load();
        });
        if (!got || g_event_queue.empty()) return 0;
    }

    QueuedRawEvent ev = std::move(g_event_queue.front());
    g_event_queue.pop();

    *out_event_type = ev.event_type;
    uint32_t n = static_cast<uint32_t>(ev.data.size());
    if (n > buf_cap) n = buf_cap;
    if (n > 0) {
        std::memcpy(out_buf, ev.data.data(), n);
    }
    if (out_len) *out_len = n;
    return 1;
}

} /* extern "C" */

/* mt5_bridge.cpp
 *
 * Named-pipe CLIENT loaded by Rust via libloading.
 * Implements protocol v6 full-duplex client with dedicated background reader thread,
 * asynchronous event dispatching, and synchronous request/response multiplexing.
 *
 * Build (MinGW-w64):
 *   x86_64-w64-mingw32-g++ -O2 -shared -o mt5_bridge.dll mt5_bridge.cpp
 *       -DMT5_BRIDGE_EXPORTS -std=c++17 -lkernel32
 */

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <unistd.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <fcntl.h>
#include <cstdint>
#include <cstring>
#include <cstdlib>
typedef void* HANDLE;
#define INVALID_HANDLE_VALUE ((HANDLE)(intptr_t)-1)
typedef uint32_t DWORD;
typedef uint32_t ULONG;
typedef int BOOL;
#define TRUE 1
#define FALSE 0
inline void Sleep(DWORD ms) { usleep(ms * 1000); }
inline DWORD GetEnvironmentVariableA(const char* name, char* buf, DWORD sz) {
    const char* val = getenv(name);
    if (!val) return 0;
    size_t len = strlen(val);
    if (len >= sz) return (DWORD)len + 1;
    memcpy(buf, val, len + 1);
    return (DWORD)len;
}
inline DWORD GetLastError() { return 0; }
inline DWORD GetCurrentProcessId() { return (DWORD)getpid(); }
inline BOOL WriteFile(HANDLE h, const void* buf, DWORD len, DWORD* written, void*) {
    int fd = (int)(intptr_t)h;
    ssize_t n = write(fd, buf, len);
    if (n < 0) return FALSE;
    if (written) *written = (DWORD)n;
    return TRUE;
}
inline BOOL ReadFile(HANDLE h, void* buf, DWORD len, DWORD* read, void*) {
    int fd = (int)(intptr_t)h;
    ssize_t n = ::read(fd, buf, len);
    if (n < 0) return FALSE;
    if (read) *read = (DWORD)n;
    return TRUE;
}
inline BOOL CloseHandle(HANDLE h) {
    int fd = (int)(intptr_t)h;
    if (fd >= 0) close(fd);
    return TRUE;
}
inline BOOL CancelIoEx(HANDLE, void*) {
    return TRUE;
}
#endif

#include <cstring>
#include <cstdint>
#include <cstdlib>
#include <string>
#include <vector>
#include <queue>
#include <unordered_map>
#include <memory>
#include <thread>
#include <mutex>
#include <condition_variable>
#include <atomic>
#include <chrono>
#include "mt5_bridge.h"

/* ── Protocol ────────────────────────────────────────────────────────────── */
/*
 * Protocol v6: Unified 16-byte packet header:
 *   [uint32 length][uint8 kind][uint8 _pad][uint16 id][int32 status][uint32 request_id]
 *
 * kind:
 *   0 = PKT_REQUEST  : id = Cmd, status = 0, length = payload_size, request_id = unique req id
 *   1 = PKT_RESPONSE : id = Cmd (matching request), status >= 0 (success/count) or < 0 (err), request_id matching
 *   2 = PKT_EVENT    : id = EventKind (TICK/TRADE/BOOK), status = 0, length = payload_size, request_id = 0
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

enum class Outcome {
    RESPONDED,
    NOT_CONNECTED,
    SEND_FAILED,
    UNKNOWN,
};

/* ── Event Queue & Observability ─────────────────────────────────────────── */

struct QueuedRawEvent {
    uint16_t    event_type;
    std::string data;
};

static const size_t               MAX_EVENT_QUEUE = 8192;
static std::queue<QueuedRawEvent> g_event_queue;
static std::mutex                 g_event_mutex;
static std::condition_variable    g_event_cv;
static Mt5EventCallback           g_event_callback = nullptr;
static std::atomic<uint64_t>      g_events_dropped_total{0};
static std::atomic<bool>          g_stopping{false};

/* ── Pending Request Table ────────────────────────────────────────────────── */

struct PendingRequest {
    std::mutex              mu;
    std::condition_variable cv;
    bool                    done = false;
    bool                    aborted = false;
    int32_t                 status = 0;
    std::string             data;
    uint16_t                cmd = 0;
};

/* ── Connection Object ──────────────────────────────────────────────────────── */

class Connection : public std::enable_shared_from_this<Connection> {
public:
    explicit Connection(HANDLE pipe) : pipe_(pipe) {}

    ~Connection() {
        kill();
        if (reader_thread_.joinable()) {
            reader_thread_.join();
        }
        if (pipe_ != INVALID_HANDLE_VALUE) {
            CloseHandle(pipe_);
            pipe_ = INVALID_HANDLE_VALUE;
        }
    }

    void start_reader() {
        reader_thread_ = std::thread(&Connection::reader_loop, this);
    }

    bool is_alive() const {
        return alive_.load(std::memory_order_acquire);
    }

    void kill() {
        if (!alive_.exchange(false, std::memory_order_acq_rel)) {
            return;
        }

        if (pipe_ != INVALID_HANDLE_VALUE) {
#if defined(_WIN32)
            CancelIoEx(pipe_, nullptr);
#endif
        }

        // Abort all pending requests
        std::lock_guard<std::mutex> lk(pending_mu_);
        for (auto& kv : pending_) {
            auto& req = kv.second;
            std::lock_guard<std::mutex> rlk(req->mu);
            req->aborted = true;
            req->done = true;
            req->cv.notify_one();
        }
        pending_.clear();

        g_event_cv.notify_all();
    }

    Outcome request(Cmd cmd, const std::string& payload,
                    int32_t& out_status, std::string& out_data,
                    int timeout_sec) {
        if (!is_alive() || pipe_ == INVALID_HANDLE_VALUE) {
            out_status = MT5_ERR_PIPE_DISCONNECTED;
            return Outcome::NOT_CONNECTED;
        }

        uint32_t req_id = next_req_id_.fetch_add(1, std::memory_order_relaxed);
        if (req_id == 0) {
            req_id = next_req_id_.fetch_add(1, std::memory_order_relaxed);
        }

        auto req = std::make_shared<PendingRequest>();
        req->cmd = static_cast<uint16_t>(cmd);

        {
            std::lock_guard<std::mutex> lk(pending_mu_);
            if (!is_alive()) {
                out_status = MT5_ERR_PIPE_DISCONNECTED;
                return Outcome::NOT_CONNECTED;
            }
            pending_[req_id] = req;
        }

        PacketHdr hdr{};
        hdr.length     = static_cast<uint32_t>(payload.size());
        hdr.kind       = PKT_REQUEST;
        hdr._pad       = 0;
        hdr.id         = static_cast<uint16_t>(cmd);
        hdr.status     = 0;
        hdr.request_id = req_id;

        // Serialize full-frame writes under write_mu_
        bool write_ok = false;
        {
            std::lock_guard<std::mutex> write_lk(write_mu_);
            if (!is_alive()) {
                std::lock_guard<std::mutex> plk(pending_mu_);
                pending_.erase(req_id);
                out_status = MT5_ERR_PIPE_DISCONNECTED;
                return Outcome::NOT_CONNECTED;
            }
            write_ok = write_frame(hdr, payload);
        }

        if (!write_ok) {
            kill();
            std::lock_guard<std::mutex> plk(pending_mu_);
            pending_.erase(req_id);
            out_status = MT5_ERR_SEND_FAILED;
            return Outcome::SEND_FAILED;
        }

        // Wait for matching response on this request's private condition variable
        std::unique_lock<std::mutex> lk(req->mu);
        bool ok = req->cv.wait_for(lk, std::chrono::seconds(timeout_sec), [&]() {
            return req->done || req->aborted || !is_alive();
        });

        {
            std::lock_guard<std::mutex> plk(pending_mu_);
            pending_.erase(req_id);
        }

        if (req->aborted || !is_alive()) {
            out_status = MT5_ERR_UNKNOWN_EXECUTION;
            return Outcome::UNKNOWN;
        }

        if (!ok || !req->done) {
            out_status = MT5_ERR_UNKNOWN_EXECUTION;
            return Outcome::UNKNOWN;
        }

        out_status = req->status;
        out_data   = std::move(req->data);
        return Outcome::RESPONDED;
    }

    uint64_t orphan_count() const {
        return orphans_.load(std::memory_order_relaxed);
    }

private:
    HANDLE                          pipe_ = INVALID_HANDLE_VALUE;
    std::atomic<bool>               alive_{true};
    std::mutex                      write_mu_;
    std::mutex                      pending_mu_;
    std::unordered_map<uint32_t, std::shared_ptr<PendingRequest>> pending_;
    std::atomic<uint32_t>           next_req_id_{1};
    std::atomic<uint64_t>           orphans_{0};
    std::thread                     reader_thread_;

    bool write_raw(const void* data, DWORD len) {
        if (pipe_ == INVALID_HANDLE_VALUE) return false;
        const char* p = static_cast<const char*>(data);
        DWORD done = 0;
        while (done < len) {
            DWORD n = 0;
            if (!WriteFile(pipe_, p + done, len - done, &n, nullptr) || n == 0) {
                return false;
            }
            done += n;
        }
        return true;
    }

    bool read_raw(void* data, DWORD len) {
        if (pipe_ == INVALID_HANDLE_VALUE) return false;
        char* p = static_cast<char*>(data);
        DWORD done = 0;
        while (done < len) {
            DWORD n = 0;
            if (!ReadFile(pipe_, p + done, len - done, &n, nullptr) || n == 0) {
                return false;
            }
            done += n;
        }
        return true;
    }

    bool write_frame(const PacketHdr& hdr, const std::string& payload) {
        if (!write_raw(&hdr, sizeof(hdr))) return false;
        if (!payload.empty() && !write_raw(payload.data(), static_cast<DWORD>(payload.size()))) return false;
        return true;
    }

    void reader_loop() {
        while (alive_.load(std::memory_order_acquire)) {
            PacketHdr hdr{};
            if (!read_raw(&hdr, sizeof(hdr))) {
                break;
            }

            if (hdr.length > MT5_MAX_FRAME_BYTES) {
                // Protocol violation: kill connection rather than stalling in zombie state
                break;
            }

            std::string payload;
            if (hdr.length > 0) {
                payload.resize(hdr.length);
                if (!read_raw(&payload[0], hdr.length)) {
                    break;
                }
            }

            if (hdr.kind == PKT_RESPONSE) {
                std::shared_ptr<PendingRequest> target;
                {
                    std::lock_guard<std::mutex> lk(pending_mu_);
                    auto it = pending_.find(hdr.request_id);
                    if (it != pending_.end() && it->second->cmd == hdr.id) {
                        target = it->second;
                    }
                }

                if (target) {
                    std::lock_guard<std::mutex> rlk(target->mu);
                    target->status = hdr.status;
                    target->data   = std::move(payload);
                    target->done   = true;
                    target->cv.notify_one();
                } else {
                    // Late or orphaned reply
                    orphans_.fetch_add(1, std::memory_order_relaxed);
                }
            } else if (hdr.kind == PKT_EVENT) {
                Mt5EventCallback cb = nullptr;
                {
                    std::lock_guard<std::mutex> lk(g_event_mutex);
                    cb = g_event_callback;
                    if (g_event_queue.size() >= MAX_EVENT_QUEUE) {
                        g_event_queue.pop();
                        g_events_dropped_total.fetch_add(1, std::memory_order_relaxed);
                    }
                    g_event_queue.push({hdr.id, std::move(payload)});
                    g_event_cv.notify_one();
                }
                if (cb) {
                    cb(hdr.id, payload.data(), static_cast<uint32_t>(payload.size()));
                }
            }
        }

        kill();
    }
};

static std::mutex                   g_conn_mu;
static std::shared_ptr<Connection>  g_conn;
static std::mutex                   g_init_mu;

static std::shared_ptr<Connection> current_conn() {
    std::lock_guard<std::mutex> lk(g_conn_mu);
    return g_conn;
}

/* ── Payload builder ─────────────────────────────────────────────────────── */

struct Packer {
    std::string buf;

    void u8 (uint8_t  v) { append(&v, 1); }
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

int IsConnected(void) {
    auto c = current_conn();
    return (c && c->is_alive()) ? 1 : 0;
}

uint64_t GetOrphanCount(void) {
    auto c = current_conn();
    return c ? c->orphan_count() : 0;
}

int Initialize(int64_t login, const char* password, const char* server) {
    std::lock_guard<std::mutex> init_lk(g_init_mu);

    // Reject re-initialization if a live connection already exists
    {
        auto existing = current_conn();
        if (existing && existing->is_alive()) {
            return MT5_ERR_ALREADY_INITIALIZED;
        }
    }

    // Teardown stale connection if any
    {
        std::lock_guard<std::mutex> lk(g_conn_mu);
        if (g_conn) {
            g_conn->kill();
            g_conn.reset();
        }
    }

    g_stopping.store(false);

    char pipe_name[256] = {0};
    DWORD env_len = GetEnvironmentVariableA("MT5_PIPE_NAME", pipe_name, sizeof(pipe_name));
    std::string pipe_path = "\\\\.\\pipe\\";
    if (env_len > 0 && env_len < sizeof(pipe_name)) {
        pipe_path += pipe_name;
    } else {
        pipe_path += "mt5bridge";
    }

    HANDLE pipe = INVALID_HANDLE_VALUE;
#if defined(_WIN32)
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

    // Verify pipe server process is not self
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
#endif

    auto conn = std::make_shared<Connection>(pipe);
    conn->start_reader();

    {
        std::lock_guard<std::mutex> lk(g_conn_mu);
        g_conn = conn;
    }

    // Clear event queue on new connection
    {
        std::lock_guard<std::mutex> lk(g_event_mutex);
        while (!g_event_queue.empty()) g_event_queue.pop();
    }

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
    Outcome out = conn->request(CMD_INIT, p.buf, st, data, 5);
    if (out != Outcome::RESPONDED || st != 1) {
        conn->kill();
        std::lock_guard<std::mutex> lk(g_conn_mu);
        if (g_conn == conn) g_conn.reset();
        return 0;
    }

    return 1;
}

int Shutdown(void) {
    g_stopping.store(true);
    auto c = current_conn();
    if (c && c->is_alive()) {
        int32_t st = 0;
        std::string data;
        c->request(CMD_SHUTDOWN, {}, st, data, 2);
        c->kill();
    }
    {
        std::lock_guard<std::mutex> lk(g_conn_mu);
        g_conn.reset();
    }
    g_event_cv.notify_all();
    return 1;
}

int CopyRates(const char* symbol, int timeframe, int64_t from,
              int64_t to, Mt5Rate* buf, int buf_capacity) {
    if (!buf || buf_capacity <= 0) return -1;
    auto c = current_conn();
    if (!c || !c->is_alive()) return -1;

    Packer p;
    p.str(symbol);
    p.i32(timeframe);
    p.i64(from);
    p.i64(to);
    p.i32(buf_capacity);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_RATES, p.buf, st, data, 60);
    if (out != Outcome::RESPONDED || st < 0) return -1;

    int32_t filled = st;
    int32_t max_n  = static_cast<int32_t>(data.size() / sizeof(Mt5Rate));
    int32_t n      = (filled < max_n) ? filled : max_n;
    if (n > buf_capacity) n = buf_capacity;
    if (n > 0) {
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Rate));
    }
    return n;
}

int AccountInfo(double* balance, double* equity,
                double* margin,  double* free_margin) {
    if (!balance || !equity || !margin || !free_margin) return 0;
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_ACCOUNT, {}, st, data, 5);
    if (out != Outcome::RESPONDED || st != 1 || data.size() < 32) return 0;

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
    auto c = current_conn();
    if (!c || !c->is_alive()) return MT5_ERR_PIPE_DISCONNECTED;

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

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_ORDER_SEND, p.buf, st, data, 20);
    switch (out) {
        case Outcome::NOT_CONNECTED:
            return MT5_ERR_PIPE_DISCONNECTED;
        case Outcome::SEND_FAILED:
            return MT5_ERR_SEND_FAILED;
        case Outcome::UNKNOWN:
            return MT5_ERR_UNKNOWN_EXECUTION;
        case Outcome::RESPONDED:
            if (result && data.size() >= sizeof(Mt5TradeResult)) {
                std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
            }
            if (st == 1) {
                return MT5_OK;
            }
            return (data.size() >= sizeof(Mt5TradeResult)) ? MT5_ERR_EA_REJECTED : MT5_ERR_GENERAL;
    }
    return MT5_ERR_UNKNOWN_EXECUTION;
}

int OrderCloseWithMagic(uint64_t ticket, uint64_t magic, Mt5TradeResult* result) {
    auto c = current_conn();
    if (!c || !c->is_alive()) return MT5_ERR_PIPE_DISCONNECTED;

    Packer p;
    p.u64(ticket);
    p.u64(magic);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_ORDER_CLOSE, p.buf, st, data, 20);
    switch (out) {
        case Outcome::NOT_CONNECTED:
            return MT5_ERR_PIPE_DISCONNECTED;
        case Outcome::SEND_FAILED:
            return MT5_ERR_SEND_FAILED;
        case Outcome::UNKNOWN:
            return MT5_ERR_UNKNOWN_EXECUTION;
        case Outcome::RESPONDED:
            if (result && data.size() >= sizeof(Mt5TradeResult)) {
                std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
            }
            if (st == 1) {
                return MT5_OK;
            }
            return (data.size() >= sizeof(Mt5TradeResult)) ? MT5_ERR_EA_REJECTED : MT5_ERR_GENERAL;
    }
    return MT5_ERR_UNKNOWN_EXECUTION;
}

int OrderClose(uint64_t ticket, Mt5TradeResult* result) {
    return OrderCloseWithMagic(ticket, 0, result);
}

int OrderModifyWithMagic(uint64_t ticket, uint64_t magic, uint8_t sl_mode, double sl, uint8_t tp_mode, double tp, Mt5TradeResult* result) {
    auto c = current_conn();
    if (!c || !c->is_alive()) return MT5_ERR_PIPE_DISCONNECTED;

    Packer p;
    p.u64(ticket);
    p.u8(sl_mode);
    p.f64(sl);
    p.u8(tp_mode);
    p.f64(tp);
    p.u64(magic);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_ORDER_MODIFY, p.buf, st, data, 20);
    switch (out) {
        case Outcome::NOT_CONNECTED:
            return MT5_ERR_PIPE_DISCONNECTED;
        case Outcome::SEND_FAILED:
            return MT5_ERR_SEND_FAILED;
        case Outcome::UNKNOWN:
            return MT5_ERR_UNKNOWN_EXECUTION;
        case Outcome::RESPONDED:
            if (result && data.size() >= sizeof(Mt5TradeResult)) {
                std::memcpy(result, data.data(), sizeof(Mt5TradeResult));
            }
            if (st == 1) {
                return MT5_OK;
            }
            return (data.size() >= sizeof(Mt5TradeResult)) ? MT5_ERR_EA_REJECTED : MT5_ERR_GENERAL;
    }
    return MT5_ERR_UNKNOWN_EXECUTION;
}

int OrderModify(uint64_t ticket, uint8_t sl_mode, double sl, uint8_t tp_mode, double tp, Mt5TradeResult* result) {
    return OrderModifyWithMagic(ticket, 0, sl_mode, sl, tp_mode, tp, result);
}

int PositionsGet(Mt5Position* buf, int buf_capacity, uint64_t magic_filter, const char* symbol_filter) {
    if (!buf || buf_capacity <= 0) return -1;
    auto c = current_conn();
    if (!c || !c->is_alive()) return -1;

    Packer p;
    p.u64(magic_filter);
    p.str(symbol_filter ? symbol_filter : "");
    p.i32(buf_capacity);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_POSITIONS_GET, p.buf, st, data, 5);
    if (out != Outcome::RESPONDED || st < 0) return -1;

    int32_t total_matching = st;
    int32_t max_items = static_cast<int32_t>(data.size() / sizeof(Mt5Position));
    int32_t n = (max_items < buf_capacity) ? max_items : buf_capacity;
    if (n > 0) {
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Position));
    }
    return total_matching;
}

int OrdersGet(Mt5Order* buf, int buf_capacity, uint64_t magic_filter, const char* symbol_filter) {
    if (!buf || buf_capacity <= 0) return -1;
    auto c = current_conn();
    if (!c || !c->is_alive()) return -1;

    Packer p;
    p.u64(magic_filter);
    p.str(symbol_filter ? symbol_filter : "");
    p.i32(buf_capacity);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_ORDERS_GET, p.buf, st, data, 5);
    if (out != Outcome::RESPONDED || st < 0) return -1;

    int32_t total_matching = st;
    int32_t max_items = static_cast<int32_t>(data.size() / sizeof(Mt5Order));
    int32_t n = (max_items < buf_capacity) ? max_items : buf_capacity;
    if (n > 0) {
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Order));
    }
    return total_matching;
}

int DealsGet(Mt5Deal* buf, int buf_capacity, int64_t from, int64_t to, uint64_t magic_filter, const char* symbol_filter) {
    if (!buf || buf_capacity <= 0) return -1;
    auto c = current_conn();
    if (!c || !c->is_alive()) return -1;

    /* Request layout (must match HandleDealsGet in the EA): from, to, magic, symbol, capacity */
    Packer p;
    p.i64(from);
    p.i64(to);
    p.u64(magic_filter);
    p.str(symbol_filter ? symbol_filter : "");
    p.i32(buf_capacity);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_DEALS_GET, p.buf, st, data, 10);
    if (out != Outcome::RESPONDED || st < 0) return -1;

    int32_t total_matching = st;
    int32_t max_items = static_cast<int32_t>(data.size() / sizeof(Mt5Deal));
    int32_t n = (max_items < buf_capacity) ? max_items : buf_capacity;
    if (n > 0) {
        std::memcpy(buf, data.data(), static_cast<size_t>(n) * sizeof(Mt5Deal));
    }
    return total_matching;
}

int SymbolInfoFull(const char* symbol, Mt5SymInfo* info) {
    if (!info) return 0;
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    Packer p;
    p.str(symbol);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_SYM_INFO, p.buf, st, data, 5);
    if (out != Outcome::RESPONDED || st != 1 || data.size() < sizeof(Mt5SymInfo))
        return 0;

    std::memcpy(info, data.data(), sizeof(Mt5SymInfo));
    return 1;
}

int SymbolInfoTick(const char* symbol, Mt5Tick* tick) {
    if (!tick) return 0;
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    Packer p;
    p.str(symbol);

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_SYM_TICK, p.buf, st, data, 5);
    if (out != Outcome::RESPONDED || st != 1 || data.size() < sizeof(Mt5Tick))
        return 0;

    std::memcpy(tick, data.data(), sizeof(Mt5Tick));
    return 1;
}

/* ── Subscription Management (Protocol v6+) ──────────────────────────────── */

int SubscribeTicks(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    Packer p;
    p.str(symbol);
    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_SUBSCRIBE_TICKS, p.buf, st, data, 5);
    return (out == Outcome::RESPONDED && st == 1) ? 1 : 0;
}

int UnsubscribeTicks(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    Packer p;
    p.str(symbol);
    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_UNSUBSCRIBE_TICKS, p.buf, st, data, 5);
    return (out == Outcome::RESPONDED && st == 1) ? 1 : 0;
}

int SubscribeTrade(void) {
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_SUBSCRIBE_TRADE, {}, st, data, 5);
    return (out == Outcome::RESPONDED && st == 1) ? 1 : 0;
}

int UnsubscribeTrade(void) {
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_UNSUBSCRIBE_TRADE, {}, st, data, 5);
    return (out == Outcome::RESPONDED && st == 1) ? 1 : 0;
}

int SubscribeBook(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    Packer p;
    p.str(symbol);
    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_SUBSCRIBE_BOOK, p.buf, st, data, 5);
    return (out == Outcome::RESPONDED && st == 1) ? 1 : 0;
}

int UnsubscribeBook(const char* symbol) {
    if (!symbol || !*symbol) return 0;
    auto c = current_conn();
    if (!c || !c->is_alive()) return 0;

    Packer p;
    p.str(symbol);
    int32_t st = 0;
    std::string data;
    Outcome out = c->request(CMD_UNSUBSCRIBE_BOOK, p.buf, st, data, 5);
    return (out == Outcome::RESPONDED && st == 1) ? 1 : 0;
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
        if (g_stopping.load(std::memory_order_acquire)) return -1;
        if (timeout_ms == 0) return 0;
        bool got = g_event_cv.wait_for(lk, std::chrono::milliseconds(timeout_ms), []() {
            return !g_event_queue.empty() || g_stopping.load(std::memory_order_acquire);
        });
        if (g_stopping.load(std::memory_order_acquire) && g_event_queue.empty()) return -1;
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

MT5_API uint64_t EventsDroppedTotal(void) {
    return g_events_dropped_total.load(std::memory_order_relaxed);
}

} /* extern "C" */

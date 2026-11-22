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


#include "../Include/Mt5Bridge/Protocol.mqh"
#include "../Include/Mt5Bridge/Auth.mqh"
#include "../Include/Mt5Bridge/Takeover.mqh"
#include "../Include/Mt5Bridge/Orders.mqh"
#include "../Include/Mt5Bridge/MarketData.mqh"
#include "../Include/Mt5Bridge/Queues.mqh"

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


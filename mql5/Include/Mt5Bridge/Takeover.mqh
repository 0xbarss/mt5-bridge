long g_server  = INVALID_HANDLE;
long g_client  = INVALID_HANDLE;
long g_takeover_event = 0;   // Win32 event handle; 0 = not created
uint g_stale_cleanup_at = 0; // GetTickCount() deadline for chart cleanup; 0 = not scheduled
bool g_stale_charts_closed = false;
bool g_running = false;
enum ENUM_INIT_STATE { INIT_WAIT_TAKEOVER, INIT_READY };
ENUM_INIT_STATE g_init_state = INIT_WAIT_TAKEOVER;
ulong g_takeover_start = 0;   // GetTickCount() when takeover waiting began

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


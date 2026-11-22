bool g_authenticated = false;
string g_pipe_path = "";
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

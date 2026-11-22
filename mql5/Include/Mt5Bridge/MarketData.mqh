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
    int total = SymbolsTotal(false);
    uchar empty[1];
    SendResponse(h, total, empty, 0);
}

// Returns a page of symbols as packed entries:
//   [u32 name_len][name bytes][Mt5SymInfo 76 bytes] repeated
// Request layout (must match SymbolsGet in bridge_dll/mt5_bridge.cpp):
//     u32 page, u32 page_size
// Response: status = total count, payload = packed entries for this page.
void HandleSymbolsGet(long h, const uchar &payload[], uint len) {
    int off = 0;
    uint page = 0, page_size = 0;
    if (!SafeUnpackU32(payload, off, len, page) ||
        !SafeUnpackU32(payload, off, len, page_size)) {
        SendError(h);
        return;
    }
    if (page_size == 0) page_size = 1000;

    int total = SymbolsTotal(false);
    uchar data[];
    int start = (int)page * (int)page_size;
    int end = MathMin(start + (int)page_size, total);
    if (start >= total) {
        uchar empty[1];
        SendResponse(h, total, empty, 0);
        return;
    }

    for (int i = start; i < end; i++) {
        string sym = SymbolName(i, false);
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
// NOTE (build 6283): CalendarValueHistory() signature is
//   (MqlCalendarValue&[], datetime, datetime, const string, const string)
// MqlCalendarValue exposes id/time/event_id/actual_value/forecast_value/
// prev_value on 6283. Each event is enriched via CalendarEventById() for
// name/importance/country_id, and country name/currency are resolved from
// CalendarCountries() by matching country_id.
void HandleCalendarEvents(long h, const uchar &payload[], uint len) {
    int off = 0;
    ulong from = 0, to = 0;
    if (!SafeUnpackU64(payload, off, len, from) ||
        !SafeUnpackU64(payload, off, len, to)) {
        SendError(h);
        return;
    }

    // Event values in the requested time window: id, time, actual_value,
    // forecast_value, prev_value (MqlCalendarValue exposes these on build 6283).
    MqlCalendarValue values[];
    int total = CalendarValueHistory(values, (datetime)from, (datetime)to, "", "");
    if (total < 0) {
        SendError(h);
        return;
    }

    // Country lookup table: country_id -> {name, code, currency}.
    MqlCalendarCountry countries[];
    int ctotal = CalendarCountries(countries);

    uchar data[];
    for (int i = 0; i < total; i++) {
        // Enrich each event value with metadata: name (title), importance,
        // country_id (MqlCalendarEvent exposes these on build 6283).
        MqlCalendarEvent event;
        bool have_event = CalendarEventById(values[i].event_id, event);

        // Resolve country name and currency from the country lookup table.
        string country = "";
        string currency = "";
        for (int k = 0; k < ctotal; k++) {
            if (countries[k].id == event.country_id) {
                country = countries[k].name;
                currency = countries[k].currency;
                break;
            }
        }

        PackU64(data, (ulong)values[i].id);
        PackI64(data, (long)values[i].time);
        PackStr(data, country);
        PackStr(data, currency);
        PackU8(data, (uchar)(have_event ? event.importance : 0));
        PackStr(data, have_event ? event.name : "");
        PackF64(data, values[i].HasActualValue() ? values[i].GetActualValue() : 0.0);
        PackU8(data, (uchar)(values[i].HasActualValue() ? 1 : 0));
        PackF64(data, values[i].HasForecastValue() ? values[i].GetForecastValue() : 0.0);
        PackU8(data, (uchar)(values[i].HasForecastValue() ? 1 : 0));
        PackF64(data, values[i].HasPreviousValue() ? values[i].GetPreviousValue() : 0.0);
        PackU8(data, (uchar)(values[i].HasPreviousValue() ? 1 : 0));
    }
    // End-of-stream terminator: id == 0.
    PackU64(data, 0);
    SendResponse(h, total, data, (uint)ArraySize(data));
}

// Returns the countries that have calendar events. build 6283 exposes
// CalendarCountries() which returns MqlCalendarCountry[] with real name/code/
// currency/currency_symbol/url_name fields.
// Response: status = country count, payload = per country:
//   [u32 name_len][name][u32 code_len][code][u32 currency_len][currency]
//   [u32 currency_symbol_len][currency_symbol][u32 url_name_len][url_name],
// terminated by name_len == 0.
void HandleCalendarCountries(long h) {
    MqlCalendarCountry countries[];
    int total = CalendarCountries(countries);
    if (total < 0) {
        SendError(h);
        return;
    }

    uchar data[];
    for (int i = 0; i < total; i++) {
        PackStr(data, countries[i].name);
        PackStr(data, countries[i].code);
        PackStr(data, countries[i].currency);
        PackStr(data, countries[i].currency_symbol);
        PackStr(data, countries[i].url_name);
    }
    PackU32(data, 0); // end-of-stream terminator (empty name)
    SendResponse(h, total, data, (uint)ArraySize(data));
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

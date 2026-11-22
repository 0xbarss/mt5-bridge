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


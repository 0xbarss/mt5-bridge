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

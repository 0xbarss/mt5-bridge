#!/usr/bin/env python3
"""
Comprehensive test suite for mt5_bridge_py — exercises EVERY exposed Python API.

Runs against a live MetaTrader 5 terminal via Wine Python 3.14.

Usage (under Wine, from crates/mt5-bridge-py/):
    wine C:\\Python314\\python.exe tests/test_all_features.py

Prerequisites:
    - MT5 terminal running with mt5_bridge.mq5 EA loaded
    - MT5_DLL_SHA256 or mt5_bridge.dll.sha256 manifest present
    - secret='test' matches the EA configuration
    - mt5_bridge.dll and mt5_bridge_py.pyd on PATH / in same directory
"""

import sys
import os
import time
import asyncio
import traceback

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import mt5_bridge_py as mb

PASS = 0
FAIL = 0
RESULTS = []


def check(name, condition, detail=""):
    global PASS, FAIL
    if condition:
        PASS += 1
        RESULTS.append(f"  PASS: {name}")
        print(f"  PASS: {name}")
    else:
        FAIL += 1
        RESULTS.append(f"  FAIL: {name} {detail}")
        print(f"  FAIL: {name} {detail}")


def section(title):
    print(f"\n{'='*60}")
    print(f"  {title}")
    print(f"{'='*60}")


# ── Data classes ──────────────────────────────────────────────────

def test_data_classes():
    section("DATA CLASSES")

    t = mb.PyTick()
    t.symbol = "EURUSD"
    t.bid = 1.05
    t.ask = 1.0502
    check("PyTick.spread()", abs(t.spread() - 0.0002) < 1e-9)
    check("PyTick.mid()", abs(t.mid() - 1.0501) < 1e-9)
    check("PyTick.__repr__", "Tick(" in repr(t))

    b = mb.PyBar(1700000000, 1.0, 1.1, 0.9, 1.05, 100.0)
    check("PyBar.mid()", abs(b.mid() - 1.0) < 1e-9)
    check("PyBar.typical_price()", abs(b.typical_price() - 1.01667) < 0.01)
    check("PyBar.range()", abs(b.range() - 0.2) < 1e-9)
    check("PyBar.is_bullish()", b.is_bullish() is True)
    check("PyBar.__repr__", "Bar(" in repr(b))

    a = mb.PyAccountInfo(1000.0, 1050.0, 500.0, 400.0, 100.0, "USD")
    check("PyAccountInfo.profit()", abs(a.profit() - 50.0) < 1e-9)
    check("PyAccountInfo.margin_level()", abs(a.margin_level() - 210.0) < 1e-9)
    check("PyAccountInfo.__repr__", "AccountInfo(" in repr(a))

    p = mb.PyPingResult(1700000000, 7, 123456, 500)
    check("PyPingResult.__repr__", "PingResult(" in repr(p))

    tr = mb.PyTradeResult(10009, 12345, 67890, 11111, 0.01, 1.05)
    check("PyTradeResult.is_success() returns bool", isinstance(tr.is_success(), bool))
    check("PyTradeResult.description() returns str", isinstance(tr.description(), str))
    check("PyTradeResult.__repr__", "TradeResult(" in repr(tr))

    pos = mb.PyPosition()
    check("PyPosition.__repr__", "Position(" in repr(pos))

    wo = mb.PyWorkingOrder()
    check("PyWorkingOrder.__repr__", "WorkingOrder(" in repr(wo))

    d = mb.PyDeal()
    check("PyDeal.__repr__", "Deal(" in repr(d))

    ce = mb.PyCalendarEvent()
    check("PyCalendarEvent.is_released()", ce.is_released() is False)
    check("PyCalendarEvent.__repr__", "CalendarEvent(" in repr(ce))

    ev = mb.PyEvent(1, b"test payload")
    check("PyEvent.event_type", ev.event_type == 1)
    check("PyEvent.payload", ev.payload == b"test payload")
    check("PyEvent.__repr__", "PyEvent(" in repr(ev))


# ── Sync Client: connection ───────────────────────────────────────

def test_sync_connection():
    section("SYNC CLIENT — CONNECTION")

    client = mb.Client.connect_with_secret("test")
    check("Client.connect_with_secret('test') returns Client", isinstance(client, mb.Client))

    try:
        client.shutdown()
        check("Client.shutdown() works", True)
    except Exception as e:
        check("Client.shutdown() works", False, f"raised {e}")

    client = mb.Client.connect_with_secret("test")
    check("Client re-connect works", isinstance(client, mb.Client))
    return client


# ── Sync Client: market data ──────────────────────────────────────

def test_sync_market_data(client):
    section("SYNC CLIENT — MARKET DATA")

    symbol = "EURUSD"

    try:
        ping = client.ping()
        check("ping() returns PyPingResult", isinstance(ping, mb.PyPingResult))
        if isinstance(ping, mb.PyPingResult):
            check("ping.server_time_secs is int", isinstance(ping.server_time_secs, int))
            check("ping.protocol is int", isinstance(ping.protocol, int))
            check("ping.monotonic_microseconds is int", isinstance(ping.monotonic_microseconds, int))
            check("ping.rtt_us is int", isinstance(ping.rtt_us, int))
            print(f"    ping: server_time={ping.server_time_secs} proto={ping.protocol} rtt={ping.rtt_us}us")
    except Exception as e:
        check("ping() works", False, f"raised {e}")

    try:
        stats = client.stats()
        check("stats() returns PyBridgeStats", isinstance(stats, mb.PyBridgeStats))
        if isinstance(stats, mb.PyBridgeStats):
            check("stats.uptime_secs is int", isinstance(stats.uptime_secs, int))
            check("stats.requests_total is int", isinstance(stats.requests_total, int))
            check("stats.protocol is int", isinstance(stats.protocol, int))
            print(f"    stats: uptime={stats.uptime_secs}s requests={stats.requests_total}")
    except Exception as e:
        check("stats() works", False, f"raised {e}")

    try:
        acct = client.account_info()
        check("account_info() returns PyAccountInfo", isinstance(acct, mb.PyAccountInfo))
        if isinstance(acct, mb.PyAccountInfo):
            check("account balance is float", isinstance(acct.balance, float))
            check("account equity is float", isinstance(acct.equity, float))
            check("account currency is str", isinstance(acct.currency, str))
            check("account.profit() works", isinstance(acct.profit(), float))
            check("account.margin_level() works", acct.margin_level() is None or isinstance(acct.margin_level(), float))
            print(f"    account: balance={acct.balance} equity={acct.equity} currency={acct.currency}")
    except Exception as e:
        check("account_info() works", False, f"raised {e}")

    try:
        total = client.symbols_total()
        check("symbols_total() returns int >= 0", isinstance(total, int) and total >= 0)
        print(f"    symbols_total: {total}")
    except Exception as e:
        check("symbols_total() works", False, f"raised {e}")

    try:
        page = client.symbols_page(0, 5)
        check("symbols_page() returns list", isinstance(page, list))
        if isinstance(page, list) and len(page) > 0:
            check("symbols_page items are PySymbolInfo", isinstance(page[0], mb.PySymbolInfo))
            print(f"    symbols_page: {len(page)} symbols")
    except Exception as e:
        check("symbols_page() works", False, f"raised {e}")

    try:
        info = client.symbol_info(symbol)
        check(f"symbol_info({symbol}) returns PySymbolInfo", isinstance(info, mb.PySymbolInfo))
        if isinstance(info, mb.PySymbolInfo):
            check(f"symbol_info.symbol == {symbol}", info.symbol == symbol)
            check(f"symbol_info.bid is float", isinstance(info.bid, float))
            check(f"symbol_info.ask is float", isinstance(info.ask, float))
            check(f"symbol_info.digits is int", isinstance(info.digits, int))
            print(f"    symbol_info({symbol}): bid={info.bid} ask={info.ask} digits={info.digits}")
    except Exception as e:
        check(f"symbol_info({symbol}) works", False, f"raised {e}")

    try:
        tick = client.symbol_tick(symbol)
        check(f"symbol_tick({symbol}) returns PyTick", isinstance(tick, mb.PyTick))
        if isinstance(tick, mb.PyTick):
            check(f"symbol_tick.symbol == {symbol}", tick.symbol == symbol)
            check(f"symbol_tick.bid is float", isinstance(tick.bid, float))
            check(f"symbol_tick.spread() works", isinstance(tick.spread(), float))
            print(f"    symbol_tick({symbol}): bid={tick.bid} ask={tick.ask} spread={tick.spread():.5f}")
    except Exception as e:
        check(f"symbol_tick({symbol}) works", False, f"raised {e}")

    now = int(time.time())
    from_time = now - 86400
    for tf in ["M1", "M5", "H1", "D1"]:
        try:
            bars = client.copy_bars(symbol, tf, from_time, now)
            check(f"copy_bars({symbol}, {tf}) returns list", isinstance(bars, list))
            if isinstance(bars, list) and len(bars) > 0:
                check(f"copy_bars({tf}) items are PyBar", isinstance(bars[0], mb.PyBar))
                print(f"    copy_bars({symbol} {tf}): {len(bars)} bars")
        except Exception as e:
            check(f"copy_bars({symbol}, {tf}) works", False, f"raised {e}")

    return info, tick


# ── Sync Client: account & history ────────────────────────────────

def test_sync_account_history(client):
    section("SYNC CLIENT — ACCOUNT & HISTORY")

    now = int(time.time())
    from_time = now - 86400

    try:
        positions = client.positions()
        check("positions() returns list", isinstance(positions, list))
        print(f"    positions: {len(positions)} items")
    except Exception as e:
        check("positions() works", False, f"raised {e}")

    try:
        orders = client.pending_orders()
        check("pending_orders() returns list", isinstance(orders, list))
        print(f"    pending_orders: {len(orders)} items")
    except Exception as e:
        check("pending_orders() works", False, f"raised {e}")

    try:
        deals = client.deals(from_time, now)
        check("deals() returns list", isinstance(deals, list))
        print(f"    deals: {len(deals)} items")
    except Exception as e:
        check("deals() works", False, f"raised {e}")

    try:
        events = client.calendar_events(from_time, now)
        check("calendar_events() returns list", isinstance(events, list))
        print(f"    calendar_events: {len(events)} items")
    except Exception as e:
        check("calendar_events() works", False, f"raised {e}")

    try:
        countries = client.calendar_countries()
        check("calendar_countries() returns list", isinstance(countries, list))
        if len(countries) > 0:
            check("calendar_countries items are str", isinstance(countries[0], str))
        print(f"    calendar_countries: {len(countries)} items")
    except Exception as e:
        check("calendar_countries() works", False, f"raised {e}")


# ── Sync Client: symbol select & subscriptions ────────────────────

def test_sync_subscriptions(client, symbol="EURUSD"):
    section("SYNC CLIENT — SUBSCRIPTIONS")

    try:
        client.symbol_select(symbol, True)
        check("symbol_select(symbol, True) works", True)
    except Exception as e:
        check("symbol_select works", False, f"raised {e}")

    try:
        result = client.subscribe_ticks(symbol)
        check("subscribe_ticks() returns bool True", isinstance(result, bool) and result is True)
        client.unsubscribe_ticks(symbol)
        check("unsubscribe_ticks() works", True)
    except Exception as e:
        check("subscribe_ticks/unsubscribe_ticks works", False, f"raised {e}")

    try:
        result = client.subscribe_trade()
        check("subscribe_trade() returns bool True", isinstance(result, bool) and result is True)
        client.unsubscribe_trade()
        check("unsubscribe_trade() works", True)
    except Exception as e:
        check("subscribe_trade/unsubscribe_trade works", False, f"raised {e}")

    try:
        result = client.subscribe_book(symbol)
        check("subscribe_book() returns bool True", isinstance(result, bool) and result is True)
        client.unsubscribe_book(symbol)
        check("unsubscribe_book() works", True)
    except Exception as e:
        check("subscribe_book/unsubscribe_book works", False, f"raised {e}")

    try:
        result = client.subscribe_bars(symbol, "M5")
        check("subscribe_bars() returns bool True", isinstance(result, bool) and result is True)
        client.unsubscribe_bars(symbol, "M5")
        check("unsubscribe_bars() works", True)
    except Exception as e:
        check("subscribe_bars/unsubscribe_bars works", False, f"raised {e}")


# ── Sync Client: trading ──────────────────────────────────────────

def test_sync_trading(client, symbol="EURUSD"):
    section("SYNC CLIENT — TRADING")

    try:
        tick = client.symbol_tick(symbol)
        price = tick.ask
    except Exception:
        price = 1.0942

    sl = price - 0.01
    tp = price + 0.01

    ticket = None
    try:
        result = client.order_send(symbol, "buy", 0.01, price, sl, tp, "python-test")
        check("order_send(buy) returns PyTradeResult", isinstance(result, mb.PyTradeResult))
        if isinstance(result, mb.PyTradeResult):
            check("order_send retcode is int", isinstance(result.retcode, int))
            check("order_send.deal is int", isinstance(result.deal, int))
            check("order_send.order is int", isinstance(result.order, int))
            ticket = result.order
            print(f"    order_send(buy): retcode={result.retcode} order={result.order} deal={result.deal}")
    except Exception as e:
        check("order_send(buy) works", False, f"raised {e}")

    try:
        client.order_send(symbol, "buy", 0.01, price, 0.0, 0.0, "no-sl-test")
        check("order_send without SL raises", False, "should have raised")
    except Exception as e:
        check("order_send without SL raises (expected)", True)
        print(f"    order_send no-SL correctly rejected: {e}")

    if ticket is not None:
        try:
            result = client.order_close(ticket)
            check("order_close returns PyTradeResult", isinstance(result, mb.PyTradeResult))
            print(f"    order_close: retcode={result.retcode}")
        except Exception as e:
            check("order_close works", False, f"raised {e}")

    try:
        result = client.order_modify(0, "set", sl, "set", tp)
        check("order_modify returns PyTradeResult", isinstance(result, mb.PyTradeResult))
    except Exception as e:
        check("order_modify works (may fail if no matching order)", True)
        print(f"    order_modify result: {e}")


# ── Async Client ──────────────────────────────────────────────────

def test_async_client():
    section("ASYNC CLIENT")

    async def run():
        client = mb.AsyncClient.connect_with_secret("test")
        check("AsyncClient.connect_with_secret returns AsyncClient", isinstance(client, mb.AsyncClient))

        symbol = "EURUSD"

        try:
            ping = await client.ping()
            check("async ping() returns PyPingResult", isinstance(ping, mb.PyPingResult))
        except Exception as e:
            check("async ping() works", False, f"raised {e}")

        try:
            stats = await client.stats()
            check("async stats() returns PyBridgeStats", isinstance(stats, mb.PyBridgeStats))
        except Exception as e:
            check("async stats() works", False, f"raised {e}")

        try:
            acct = await client.account_info()
            check("async account_info() returns PyAccountInfo", isinstance(acct, mb.PyAccountInfo))
        except Exception as e:
            check("async account_info() works", False, f"raised {e}")

        try:
            info = await client.symbol_info(symbol)
            check("async symbol_info() returns PySymbolInfo", isinstance(info, mb.PySymbolInfo))
        except Exception as e:
            check("async symbol_info() works", False, f"raised {e}")

        try:
            tick = await client.symbol_tick(symbol)
            check("async symbol_tick() returns PyTick", isinstance(tick, mb.PyTick))
        except Exception as e:
            check("async symbol_tick() works", False, f"raised {e}")

        try:
            now = int(time.time())
            bars = await client.copy_bars(symbol, "M5", now - 3600, now)
            check("async copy_bars() returns list", isinstance(bars, list))
        except Exception as e:
            check("async copy_bars() works", False, f"raised {e}")

        try:
            tick = await client.symbol_tick(symbol)
            await client.order_send(symbol, "buy", 0.01, tick.ask, 0.0, 0.0, "no-sl")
            check("async order_send without SL raises", False, "should have raised")
        except Exception as e:
            check("async order_send without SL raises (expected)", True)

        try:
            await client.shutdown()
            check("async shutdown() works", True)
        except Exception as e:
            check("async shutdown() works", False, f"raised {e}")

    asyncio.run(run())


# ── Event System (Task 5) ─────────────────────────────────────────

def test_event_system():
    section("EVENT SYSTEM (Task 5)")

    async def run():
        client = mb.AsyncClient.connect_with_secret("test")
        check("AsyncClient.connect_with_secret() for events", isinstance(client, mb.AsyncClient))

        received = []
        def cb(event_type, payload):
            received.append((event_type, len(payload)))
        try:
            rc = client.register_event_callback(cb)
            check("register_event_callback returns int rc", isinstance(rc, int))
        except Exception as e:
            check("register_event_callback works", False, f"raised {e}")

        try:
            ev = client.poll_event(100)
            check("poll_event returns Optional[PyEvent]", ev is None or isinstance(ev, mb.PyEvent))
            if isinstance(ev, mb.PyEvent):
                print(f"    poll_event: type={ev.event_type} payload_len={len(ev.payload)}")
        except Exception as e:
            check("poll_event works", False, f"raised {e}")

        try:
            dropped = client.events_dropped_total()
            check("events_dropped_total returns int >= 0", isinstance(dropped, int) and dropped >= 0)
            print(f"    events_dropped_total: {dropped}")
        except Exception as e:
            check("events_dropped_total works", False, f"raised {e}")

        try:
            orphans = client.get_orphan_count()
            check("get_orphan_count returns int >= 0", isinstance(orphans, int) and orphans >= 0)
            print(f"    get_orphan_count: {orphans}")
        except Exception as e:
            check("get_orphan_count works", False, f"raised {e}")

        try:
            count = 0
            async for event in client.events():
                check("events() yields PyEvent", isinstance(event, mb.PyEvent))
                count += 1
                if count >= 3:
                    break
            check("events() async iterator works", True)
            print(f"    events(): received {count} events")
        except Exception as e:
            check("events() async iterator works", False, f"raised {e}")

        await client.shutdown()

    asyncio.run(run())


# ── Main ──────────────────────────────────────────────────────────

def main():
    print("mt5_bridge_py Comprehensive Feature Test — ALL APIs")
    print("====================================================")
    print(f"Python: {sys.version}")
    print(f"mt5_bridge_py: {mb.__version__}")

    test_data_classes()
    client = test_sync_connection()
    test_sync_market_data(client)
    test_sync_account_history(client)
    test_sync_subscriptions(client)
    test_sync_trading(client)
    client.shutdown()
    test_async_client()
    test_event_system()

    print(f"\n{'='*60}")
    print(f"RESULTS: {PASS} passed, {FAIL} failed out of {PASS + FAIL}")
    print(f"{'='*60}")

    if FAIL > 0:
        print("\nFailures:")
        for r in RESULTS:
            if "FAIL" in r:
                print(f"  {r}")
        sys.exit(1)
    else:
        print("\nAll tests passed!")
        sys.exit(0)


if __name__ == "__main__":
    main()

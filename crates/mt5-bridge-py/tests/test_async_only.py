#!/usr/bin/env python3
"""
Async-only test suite for mt5_bridge_py — exercises the AsyncClient API.

Runs against a live MetaTrader 5 terminal via Wine Python 3.14 in its OWN
process. The DLL enforces one client per process, so this suite must NOT be
run in the same process as the sync test suite (test_all_features.py).

Usage (under Wine, from crates/mt5-bridge-py/):
    wine C:\\Python314\\python.exe tests/test_async_only.py

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


# ── Async Client: connection ──────────────────────────────────────

def test_async_connection():
    section("ASYNC CLIENT — CONNECTION")

    async def run():
        client = mb.AsyncClient.connect_with_secret("test")
        check("AsyncClient.connect_with_secret('test') returns AsyncClient", isinstance(client, mb.AsyncClient))

        try:
            await client.shutdown()
            check("AsyncClient.shutdown() works", True)
        except Exception as e:
            check("AsyncClient.shutdown() works", False, f"raised {e}")

        client = mb.AsyncClient.connect_with_secret("test")
        check("AsyncClient re-connect works", isinstance(client, mb.AsyncClient))

        try:
            await client.shutdown()
            check("AsyncClient second shutdown() works", True)
        except Exception as e:
            check("AsyncClient second shutdown() works", False, f"raised {e}")

    asyncio.run(run())


# ── Async Client: market data ─────────────────────────────────────

def test_async_market_data():
    section("ASYNC CLIENT — MARKET DATA")

    symbol = "EURUSD"

    async def run():
        client = mb.AsyncClient.connect_with_secret("test")

        try:
            ping = await client.ping()
            check("async ping() returns PyPingResult", isinstance(ping, mb.PyPingResult))
            if isinstance(ping, mb.PyPingResult):
                check("ping.server_time_secs is int", isinstance(ping.server_time_secs, int))
                check("ping.protocol is int", isinstance(ping.protocol, int))
                check("ping.monotonic_microseconds is int", isinstance(ping.monotonic_microseconds, int))
                check("ping.rtt_us is int", isinstance(ping.rtt_us, int))
                print(f"    ping: server_time={ping.server_time_secs} proto={ping.protocol} rtt={ping.rtt_us}us")
        except Exception as e:
            check("async ping() works", False, f"raised {e}")

        try:
            stats = await client.stats()
            check("async stats() returns PyBridgeStats", isinstance(stats, mb.PyBridgeStats))
            if isinstance(stats, mb.PyBridgeStats):
                check("stats.uptime_secs is int", isinstance(stats.uptime_secs, int))
                check("stats.requests_total is int", isinstance(stats.requests_total, int))
                check("stats.protocol is int", isinstance(stats.protocol, int))
                print(f"    stats: uptime={stats.uptime_secs}s requests={stats.requests_total}")
        except Exception as e:
            check("async stats() works", False, f"raised {e}")

        try:
            acct = await client.account_info()
            check("async account_info() returns PyAccountInfo", isinstance(acct, mb.PyAccountInfo))
            if isinstance(acct, mb.PyAccountInfo):
                check("account balance is float", isinstance(acct.balance, float))
                check("account equity is float", isinstance(acct.equity, float))
                check("account currency is str", isinstance(acct.currency, str))
                check("account.profit() works", isinstance(acct.profit(), float))
                check("account.margin_level() works", acct.margin_level() is None or isinstance(acct.margin_level(), float))
                print(f"    account: balance={acct.balance} equity={acct.equity} currency={acct.currency}")
        except Exception as e:
            check("async account_info() works", False, f"raised {e}")

        try:
            info = await client.symbol_info(symbol)
            check(f"async symbol_info({symbol}) returns PySymbolInfo", isinstance(info, mb.PySymbolInfo))
            if isinstance(info, mb.PySymbolInfo):
                check(f"async symbol_info.symbol == {symbol}", info.symbol == symbol)
                check(f"async symbol_info.bid is float", isinstance(info.bid, float))
                check(f"async symbol_info.ask is float", isinstance(info.ask, float))
                check(f"async symbol_info.digits is int", isinstance(info.digits, int))
                print(f"    symbol_info({symbol}): bid={info.bid} ask={info.ask} digits={info.digits}")
        except Exception as e:
            check(f"async symbol_info({symbol}) works", False, f"raised {e}")

        try:
            tick = await client.symbol_tick(symbol)
            check(f"async symbol_tick({symbol}) returns PyTick", isinstance(tick, mb.PyTick))
            if isinstance(tick, mb.PyTick):
                check(f"async symbol_tick.symbol == {symbol}", tick.symbol == symbol)
                check(f"async symbol_tick.bid is float", isinstance(tick.bid, float))
                check(f"async symbol_tick.spread() works", isinstance(tick.spread(), float))
                print(f"    symbol_tick({symbol}): bid={tick.bid} ask={tick.ask} spread={tick.spread():.5f}")
        except Exception as e:
            check(f"async symbol_tick({symbol}) works", False, f"raised {e}")

        try:
            now = int(time.time())
            bars = await client.copy_bars(symbol, "M5", now - 3600, now)
            check("async copy_bars() returns list", isinstance(bars, list))
            if isinstance(bars, list) and len(bars) > 0:
                check("async copy_bars() items are PyBar", isinstance(bars[0], mb.PyBar))
                print(f"    copy_bars({symbol} M5): {len(bars)} bars")
        except Exception as e:
            check("async copy_bars() works", False, f"raised {e}")

        try:
            await client.shutdown()
            check("async shutdown() works", True)
        except Exception as e:
            check("async shutdown() works", False, f"raised {e}")

    asyncio.run(run())


# ── Async Client: trading ─────────────────────────────────────────

def test_async_trading():
    section("ASYNC CLIENT — TRADING")

    symbol = "EURUSD"

    async def run():
        client = mb.AsyncClient.connect_with_secret("test")

        try:
            tick = await client.symbol_tick(symbol)
            price = tick.ask
        except Exception:
            price = 1.0942

        try:
            await client.order_send(symbol, "buy", 0.01, price, 0.0, 0.0, "no-sl")
            check("async order_send without SL raises", False, "should have raised")
        except Exception as e:
            check("async order_send without SL raises (expected)", True)
            print(f"    order_send no-SL correctly rejected: {e}")

        try:
            await client.shutdown()
            check("async shutdown() works", True)
        except Exception as e:
            check("async shutdown() works", False, f"raised {e}")

    asyncio.run(run())


# ── Async Client: event system ────────────────────────────────────

def test_async_event_system():
    section("ASYNC CLIENT — EVENT SYSTEM")

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
        check("async shutdown() works", True)

    asyncio.run(run())


# ── Main ──────────────────────────────────────────────────────────

def main():
    print("mt5_bridge_py Async-Only Feature Test")
    print("=====================================")
    print(f"Python: {sys.version}")
    print(f"mt5_bridge_py: {mb.__version__}")

    test_data_classes()
    test_async_connection()
    test_async_market_data()
    test_async_trading()
    test_async_event_system()

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
        print("\nAll async tests passed!")
        sys.exit(0)


if __name__ == "__main__":
    main()

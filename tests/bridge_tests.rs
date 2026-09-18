use mt5_bridge::*;

#[test]
fn test_timeframe_math() {
    assert_eq!(Timeframe::M1.seconds(), 60);
    assert_eq!(Timeframe::M2.seconds(), 120);
    assert_eq!(Timeframe::M3.seconds(), 180);
    assert_eq!(Timeframe::M5.seconds(), 300);
    assert_eq!(Timeframe::M6.seconds(), 360);
    assert_eq!(Timeframe::M10.seconds(), 600);
    assert_eq!(Timeframe::M12.seconds(), 720);
    assert_eq!(Timeframe::M15.seconds(), 900);
    assert_eq!(Timeframe::M20.seconds(), 1200);
    assert_eq!(Timeframe::M30.seconds(), 1800);
    assert_eq!(Timeframe::H1.seconds(), 3600);
    assert_eq!(Timeframe::H2.seconds(), 7200);
    assert_eq!(Timeframe::H3.seconds(), 10800);
    assert_eq!(Timeframe::H4.seconds(), 14400);
    assert_eq!(Timeframe::H6.seconds(), 21600);
    assert_eq!(Timeframe::H8.seconds(), 28800);
    assert_eq!(Timeframe::H12.seconds(), 43200);
    assert_eq!(Timeframe::D1.seconds(), 86400);
    assert_eq!(Timeframe::W1.seconds(), 604800);
    assert_eq!(Timeframe::MN1.seconds(), 2592000);
}

#[test]
fn test_timeframe_calendar_intervals() {
    assert!(!Timeframe::M1.is_calendar_interval());
    assert!(!Timeframe::H1.is_calendar_interval());
    assert!(!Timeframe::D1.is_calendar_interval());
    assert!(Timeframe::W1.is_calendar_interval());
    assert!(Timeframe::MN1.is_calendar_interval());
}

#[test]
fn test_timeframe_mt5_constants() {
    assert_eq!(Timeframe::M1.to_mt5_const(), 1);
    assert_eq!(Timeframe::M5.to_mt5_const(), 5);
    assert_eq!(Timeframe::M15.to_mt5_const(), 15);
    assert_eq!(Timeframe::M30.to_mt5_const(), 30);
    assert_eq!(Timeframe::H1.to_mt5_const(), 16385);
    assert_eq!(Timeframe::H4.to_mt5_const(), 16388);
    assert_eq!(Timeframe::D1.to_mt5_const(), 16408);
    assert_eq!(Timeframe::W1.to_mt5_const(), 32769);
    assert_eq!(Timeframe::MN1.to_mt5_const(), 49153);
}

#[test]
fn test_timeframe_parsing() {
    assert_eq!("m1".parse::<Timeframe>().unwrap(), Timeframe::M1);
    assert_eq!("1m".parse::<Timeframe>().unwrap(), Timeframe::M1);
    assert_eq!("M15".parse::<Timeframe>().unwrap(), Timeframe::M15);
    assert_eq!("15m".parse::<Timeframe>().unwrap(), Timeframe::M15);
    assert_eq!("h1".parse::<Timeframe>().unwrap(), Timeframe::H1);
    assert_eq!("1h".parse::<Timeframe>().unwrap(), Timeframe::H1);
    assert_eq!("4h".parse::<Timeframe>().unwrap(), Timeframe::H4);
    assert_eq!("d1".parse::<Timeframe>().unwrap(), Timeframe::D1);
    assert_eq!("1d".parse::<Timeframe>().unwrap(), Timeframe::D1);
    assert_eq!("w1".parse::<Timeframe>().unwrap(), Timeframe::W1);
    assert_eq!("1w".parse::<Timeframe>().unwrap(), Timeframe::W1);
    assert_eq!("mn1".parse::<Timeframe>().unwrap(), Timeframe::MN1);
    assert_eq!("1mn".parse::<Timeframe>().unwrap(), Timeframe::MN1);
    assert!("invalid".parse::<Timeframe>().is_err());
}

#[test]
fn test_round_lot_edge_cases() {
    let sym = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.0,
        digits: 5,
    };

    // Below min_lot should return 0.0 to prevent risk inflation
    assert_eq!(sym.round_lot(0.005), 0.0);
    assert_eq!(sym.round_lot(0.0), 0.0);
    assert_eq!(sym.round_lot(-1.0), -1.0);
    assert_eq!(sym.round_lot(0.01), 0.01);
    assert_eq!(sym.round_lot(0.019), 0.02);
    assert_eq!(sym.round_lot(150.0), 100.0);
}

#[test]
fn test_is_valid_lot() {
    let sym = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 10.0,
        spread: 1.0,
        digits: 5,
    };

    assert!(sym.is_valid_lot(0.01));
    assert!(sym.is_valid_lot(0.55));
    assert!(sym.is_valid_lot(10.0));
    assert!(!sym.is_valid_lot(0.005)); // Below min
    assert!(!sym.is_valid_lot(10.01)); // Above max
    assert!(!sym.is_valid_lot(0.015)); // Off step
    assert!(!sym.is_valid_lot(0.0));
    assert!(!sym.is_valid_lot(-1.0));
    assert!(!sym.is_valid_lot(f64::NAN));
    assert!(!sym.is_valid_lot(f64::INFINITY));
}

#[test]
fn test_round_price() {
    let sym_5d = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.0,
        digits: 5,
    };
    assert_eq!(sym_5d.round_price(1.0854321), 1.08543);
    assert_eq!(sym_5d.round_price(1.085437), 1.08544);

    let sym_3d = SymbolInfo {
        symbol: "USDJPY".to_string(),
        point: 0.001,
        tick_value: 0.65,
        tick_size: 0.001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.0,
        digits: 3,
    };
    assert_eq!(sym_3d.round_price(154.1238), 154.124);

    let sym_0d = SymbolInfo {
        symbol: "INDEX".to_string(),
        point: 1.0,
        tick_value: 1.0,
        tick_size: 1.0,
        lot_step: 0.1,
        min_lot: 0.1,
        max_lot: 100.0,
        spread: 1.0,
        digits: 0,
    };
    assert_eq!(sym_0d.round_price(45123.6), 45124.0);
}

#[test]
fn test_point_value_scaling() {
    // 1. Standard EURUSD (point == tick_size)
    let eurusd = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.0,
        digits: 5,
    };
    // 1-point move with 1 lot = (0.00001 / 0.00001) * 1.0 * 1.0 = $1.00
    assert!((eurusd.point_value(1.0) - 1.0).abs() < 1e-6);
    // 1-point move with 0.1 lot = $0.10
    assert!((eurusd.point_value(0.1) - 0.1).abs() < 1e-6);

    // 2. Instrument where tick_size != point (e.g. tick_size = 0.00005, point = 0.00001)
    let exotic = SymbolInfo {
        symbol: "EXOTIC".to_string(),
        point: 0.00001,
        tick_value: 5.0,
        tick_size: 0.00005,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 2.0,
        digits: 5,
    };
    // (0.00001 / 0.00005) * 5.0 * 1.0 = 0.2 * 5.0 = 1.0
    assert!((exotic.point_value(1.0) - 1.0).abs() < 1e-6);

    // 3. Fallback when tick_size is 0.0
    let zero_tick = SymbolInfo {
        symbol: "ZERO".to_string(),
        point: 0.01,
        tick_value: 2.0,
        tick_size: 0.0,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.0,
        digits: 2,
    };
    assert!((zero_tick.point_value(1.0) - 2.0).abs() < 1e-6);
}

#[test]
fn test_order_request_builder() {
    let buy = OrderRequest::buy("EURUSD", 0.5)
        .stop_loss(1.0800)
        .take_profit(1.0900)
        .deviation(15)
        .magic(999)
        .expiration(1780000000)
        .comment("builder_test");

    assert_eq!(buy.symbol, "EURUSD");
    assert_eq!(buy.order_type, OrderType::Buy);
    assert_eq!(buy.volume, 0.5);
    assert_eq!(buy.price, 0.0);
    assert_eq!(buy.stop_loss, 1.0800);
    assert_eq!(buy.take_profit, 1.0900);
    assert_eq!(buy.deviation, Some(15));
    assert_eq!(buy.magic, Some(999));
    assert_eq!(buy.expiration, Some(1780000000));
    assert_eq!(buy.comment, "builder_test");

    let limit = OrderRequest::pending("GBPUSD", OrderType::BuyLimit, 0.1, 1.2500);
    assert_eq!(limit.order_type, OrderType::BuyLimit);
    assert_eq!(limit.price, 1.2500);
    assert_eq!(limit.volume, 0.1);
    assert_eq!(limit.deviation, None); // default deviation is None
}

#[test]
fn test_trade_result_success_retcodes() {
    // 10009 with deal > 0 => Filled
    let res_filled = TradeResult {
        retcode: 10009, // DONE
        deal: 1001,
        order: 2001,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_filled.is_success());
    assert_eq!(res_filled.status(), TradeStatus::Filled);
    assert!(res_filled.is_filled());
    assert!(!res_filled.is_placed());
    assert!(!res_filled.is_partially_filled());

    // 10009 with deal == 0 and order > 0 => Placed (pending order)
    let res_done_pending = TradeResult {
        retcode: 10009,
        deal: 0,
        order: 2002,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_done_pending.is_success());
    assert_eq!(res_done_pending.status(), TradeStatus::Placed);
    assert!(res_done_pending.is_placed());
    assert!(!res_done_pending.is_filled());

    // 10008 PLACED
    let res_placed = TradeResult {
        retcode: 10008,
        deal: 0,
        order: 2003,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_placed.is_success());
    assert_eq!(res_placed.status(), TradeStatus::Placed);
    assert!(res_placed.is_placed());
    assert!(!res_placed.is_filled());

    // 10010 DONE_PARTIAL
    let res_partial = TradeResult {
        retcode: 10010,
        deal: 1003,
        order: 2004,
        volume: 0.05,
        price: 1.1000,
    };
    assert!(res_partial.is_success());
    assert_eq!(res_partial.status(), TradeStatus::PartiallyFilled);
    assert!(res_partial.is_partially_filled());
    assert!(!res_partial.is_filled());

    // 10019 NO_MONEY
    let res_err = TradeResult {
        retcode: 10019,
        deal: 0,
        order: 0,
        volume: 0.0,
        price: 0.0,
    };
    assert!(!res_err.is_success());
    assert_eq!(res_err.status(), TradeStatus::Rejected);
    assert_eq!(
        res_err.description(),
        "TRADE_RETCODE_NO_MONEY: There is not enough money to complete the request"
    );
}

#[test]
fn test_history_result_completeness() {
    let complete_res = HistoryResult {
        rates: vec![],
        complete: true,
        missing_ranges: vec![],
    };
    assert!(complete_res.complete);
    assert!(complete_res.is_complete());
    assert!(complete_res.missing_ranges.is_empty());

    let partial_res = HistoryResult {
        rates: vec![],
        complete: false,
        missing_ranges: vec![(1700000000, 1700010000)],
    };
    assert!(!partial_res.complete);
    assert!(!partial_res.is_complete());
    assert_eq!(partial_res.missing_ranges.len(), 1);
}

#[test]
fn test_rate_to_bar_conversion() {
    let rate = Rate {
        time: 1780000000,
        open: 1.1000,
        high: 1.1050,
        low: 1.0980,
        close: 1.1020,
        volume: 150,
        spread: 2,
        real_volume: 300,
    };

    let bar: Bar = rate.into();
    assert_eq!(bar.time, 1780000000);
    assert_eq!(bar.open, 1.1000);
    assert_eq!(bar.high, 1.1050);
    assert_eq!(bar.low, 1.0980);
    assert_eq!(bar.close, 1.1020);
    assert_eq!(bar.volume, 150.0);
    assert!((bar.mid() - 1.1015).abs() < 1e-6);
    assert!((bar.range() - 0.0070).abs() < 1e-6);
    assert!(bar.is_bullish());
    assert!(!bar.is_bearish());
}

#[test]
fn test_tick_spread() {
    let tick = Tick {
        symbol: "EURUSD".to_string(),
        time: 1780000000,
        bid: 1.14810,
        ask: 1.14825,
        last: 0.0,
        volume: 0,
        time_msc: 1780000000000,
        flags: 0,
    };
    assert!((tick.spread() - 0.00015).abs() < 1e-6);
}

#[test]
fn test_account_info_helpers() {
    let acct = AccountInfo {
        balance: 10000.0,
        equity: 10500.0,
        margin: 500.0,
        free_margin: 10000.0,
    };
    assert_eq!(acct.profit(), 500.0);
    assert_eq!(acct.margin_level(), Some(2100.0));

    let no_margin = AccountInfo {
        balance: 10000.0,
        equity: 10000.0,
        margin: 0.0,
        free_margin: 10000.0,
    };
    assert_eq!(no_margin.margin_level(), None);

    let neg_margin = AccountInfo {
        balance: 10000.0,
        equity: 10000.0,
        margin: -5.0,
        free_margin: 10000.0,
    };
    assert_eq!(neg_margin.margin_level(), None);
}

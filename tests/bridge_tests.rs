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

    // Below min_lot or invalid should return 0.0 to prevent risk inflation
    assert_eq!(sym.round_lot(0.005), 0.0);
    assert_eq!(sym.round_lot(0.0), 0.0);
    assert_eq!(sym.round_lot(-1.0), 0.0);
    assert_eq!(sym.round_lot(f64::NAN), 0.0);
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
fn test_floor_and_ceil_lot() {
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

    // floor_lot never rounds up (preserves max risk constraints)
    assert_eq!(sym.floor_lot(0.149), 0.14);
    assert_eq!(sym.floor_lot(0.019), 0.01);
    assert_eq!(sym.floor_lot(0.009), 0.0);
    assert_eq!(sym.floor_lot(15.0), 10.0);

    // ceil_lot
    assert_eq!(sym.ceil_lot(0.141), 0.15);
    assert_eq!(sym.ceil_lot(0.001), 0.01);
}

#[test]
fn test_min_lot_offset_and_unbounded_max() {
    // Symbol with min_lot offset from lot_step (e.g. min 0.05, step 0.02)
    let sym_offset = SymbolInfo {
        symbol: "US30".to_string(),
        point: 0.01,
        tick_value: 1.0,
        tick_size: 0.01,
        lot_step: 0.02,
        min_lot: 0.05,
        max_lot: 0.0, // unbounded broker max
        spread: 1.0,
        digits: 2,
    };

    assert!(!sym_offset.is_min_lot_zero_aligned());
    assert!(sym_offset.is_valid_lot(0.05));
    assert!(sym_offset.is_valid_lot(0.07));
    assert!(sym_offset.is_valid_lot(0.09));
    assert!(sym_offset.is_valid_lot(500.0)); // unbounded max allowed

    assert_eq!(sym_offset.round_lot(0.05), 0.05);
    assert_eq!(sym_offset.round_lot(0.07), 0.07);
    assert_eq!(sym_offset.round_lot(0.06), 0.07);
    assert_eq!(sym_offset.floor_lot(0.06), 0.05);
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
        position: 3001,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_filled.is_success());
    assert_eq!(res_filled.position, 3001);
    assert_eq!(res_filled.status(), TradeStatus::Filled);
    assert!(res_filled.is_filled());
    assert!(!res_filled.is_placed());
    assert!(!res_filled.is_partially_filled());

    assert!(res_filled.is_deal());
    assert!(!res_filled.is_working_order());
    assert!(res_filled.has_position());

    // 10009 with deal == 0 and order > 0 => Placed (pending order)
    let res_done_pending = TradeResult {
        retcode: 10009,
        deal: 0,
        order: 2002,
        position: 0,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_done_pending.is_success());
    assert_eq!(res_done_pending.status(), TradeStatus::Placed);
    assert!(res_done_pending.is_placed());
    assert!(!res_done_pending.is_filled());
    assert!(!res_done_pending.is_deal());
    assert!(res_done_pending.is_working_order());
    assert!(!res_done_pending.has_position());

    // 10008 PLACED
    let res_placed = TradeResult {
        retcode: 10008,
        deal: 0,
        order: 2003,
        position: 0,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_placed.is_success());
    assert_eq!(res_placed.status(), TradeStatus::Placed);
    assert!(res_placed.is_placed());
    assert!(!res_placed.is_filled());
    assert!(!res_placed.is_deal());
    assert!(res_placed.is_working_order());

    // 10010 DONE_PARTIAL
    let res_partial = TradeResult {
        retcode: 10010,
        deal: 1003,
        order: 2004,
        position: 3004,
        volume: 0.05,
        price: 1.1000,
    };
    assert!(res_partial.is_success());
    assert_eq!(res_partial.status(), TradeStatus::PartiallyFilled);
    assert!(res_partial.is_partially_filled());
    assert!(!res_partial.is_filled());
    assert!(res_partial.is_deal());
    assert!(!res_partial.is_working_order());
    assert!(res_partial.has_position());

    // 10019 NO_MONEY
    let res_err = TradeResult {
        retcode: 10019,
        deal: 0,
        order: 0,
        position: 0,
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
fn test_order_request_validate_with_symbol() {
    let sym = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00005,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.2,
        digits: 5,
    };

    // Valid request
    let req_ok = OrderRequest::buy("EURUSD", 0.10);
    assert!(req_ok.validate_with_symbol(&sym).is_ok());

    // Invalid volume step (0.015 when step is 0.01)
    let req_bad_step = OrderRequest::buy("EURUSD", 0.015);
    assert!(req_bad_step.validate_with_symbol(&sym).is_err());

    // Below min lot (0.005 when min is 0.01)
    let req_low_vol = OrderRequest::buy("EURUSD", 0.005);
    assert!(req_low_vol.validate_with_symbol(&sym).is_err());

    // Price not aligned with tick size (0.00005 tick size, price ending in 0.00003)
    let req_bad_price = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 0.10, 1.10003);
    assert!(req_bad_price.validate_with_symbol(&sym).is_err());

    // Price correctly aligned with tick size
    let req_good_price = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 0.10, 1.10005);
    assert!(req_good_price.validate_with_symbol(&sym).is_ok());
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

#[test]
fn test_protocol_version() {
    // v5: push model, EVENT framing, and subscriptions.
    assert_eq!(PROTOCOL_VERSION, 5);
}

#[test]
fn test_trade_result_from_raw() {
    use mt5_bridge::ffi::Mt5TradeResult;

    let raw = Mt5TradeResult {
        retcode: 10009,
        deal: 12345,
        order: 67890,
        position: 112233,
        volume: 0.5,
        price: 1.0850,
    };
    let res = TradeResult::from_raw(raw);
    assert_eq!(res.retcode, 10009);
    assert_eq!(res.deal, 12345);
    assert_eq!(res.order, 67890);
    assert_eq!(res.position, 112233);
    assert_eq!(res.volume, 0.5);
    assert_eq!(res.price, 1.0850);
}

#[test]
fn test_trade_result_wire_abi_layout() {
    use mt5_bridge::ffi::Mt5TradeResult;

    // Simulate the exact 44-byte binary stream generated by MQL5 serialization:
    // retcode (u32, 4) + deal (u64, 8) + order (u64, 8) + position (u64, 8) + volume (f64, 8) + price (f64, 8)
    let mut bytes = Vec::with_capacity(44);
    bytes.extend_from_slice(&10009u32.to_le_bytes());
    bytes.extend_from_slice(&111111u64.to_le_bytes());
    bytes.extend_from_slice(&222222u64.to_le_bytes());
    bytes.extend_from_slice(&333333u64.to_le_bytes());
    bytes.extend_from_slice(&0.37f64.to_le_bytes());
    bytes.extend_from_slice(&1.23456f64.to_le_bytes());

    assert_eq!(bytes.len(), 44);

    let raw: Mt5TradeResult = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Mt5TradeResult) };
    let (raw_retcode, raw_deal, raw_order, raw_position, raw_volume, raw_price) = (
        raw.retcode,
        raw.deal,
        raw.order,
        raw.position,
        raw.volume,
        raw.price,
    );
    assert_eq!(raw_retcode, 10009);
    assert_eq!(raw_deal, 111111);
    assert_eq!(raw_order, 222222);
    assert_eq!(raw_position, 333333);
    assert!((raw_volume - 0.37).abs() < 1e-6);
    assert!((raw_price - 1.23456).abs() < 1e-6);

    let res = TradeResult::from_raw(raw);
    assert_eq!(res.retcode, 10009);
    assert_eq!(res.deal, 111111);
    assert_eq!(res.order, 222222);
    assert_eq!(res.position, 333333);
    assert!((res.volume - 0.37).abs() < 1e-6);
    assert!((res.price - 1.23456).abs() < 1e-6);
    assert_eq!(res.status(), mt5_bridge::types::TradeStatus::Filled);
    assert!(res.is_success());
    assert!(res.is_filled());
}

#[test]
fn test_order_request_validation() {
    // Valid order
    let valid = OrderRequest::buy("EURUSD", 0.1)
        .stop_loss(1.08)
        .take_profit(1.10);
    assert!(valid.validate().is_ok());

    // Empty symbol
    let empty_sym = OrderRequest::buy("", 0.1);
    assert!(empty_sym.validate().is_err());
    let blank_sym = OrderRequest::buy("   ", 0.1);
    assert!(blank_sym.validate().is_err());

    // Invalid volume
    let zero_vol = OrderRequest::buy("EURUSD", 0.0);
    assert!(zero_vol.validate().is_err());
    let neg_vol = OrderRequest::buy("EURUSD", -0.1);
    assert!(neg_vol.validate().is_err());
    let nan_vol = OrderRequest::buy("EURUSD", f64::NAN);
    assert!(nan_vol.validate().is_err());
    let inf_vol = OrderRequest::buy("EURUSD", f64::INFINITY);
    assert!(inf_vol.validate().is_err());

    // Invalid prices
    let neg_price = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 0.1, -1.0);
    assert!(neg_price.validate().is_err());
    let nan_price = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 0.1, f64::NAN);
    assert!(nan_price.validate().is_err());

    // Invalid stops
    let neg_sl = OrderRequest::buy("EURUSD", 0.1).stop_loss(-1.0);
    assert!(neg_sl.validate().is_err());
    let nan_sl = OrderRequest::buy("EURUSD", 0.1).stop_loss(f64::NAN);
    assert!(nan_sl.validate().is_err());
    let neg_tp = OrderRequest::buy("EURUSD", 0.1).take_profit(-1.0);
    assert!(neg_tp.validate().is_err());
    let nan_tp = OrderRequest::buy("EURUSD", 0.1).take_profit(f64::NAN);
    assert!(nan_tp.validate().is_err());
}

#[test]
fn test_position_wire_decoding_and_helpers() {
    let mut symbol = [0u8; 32];
    symbol[..6].copy_from_slice(b"EURUSD");
    let mut comment = [0u8; 32];
    comment[..12].copy_from_slice(b"cid:ord-1234");

    let mut raw = mt5_bridge::ffi::Mt5Position {
        ticket: 1001,
        time: 1700000000,
        position_type: 0, // Buy
        magic: 998877,
        volume: 0.5,
        price_open: 1.08500,
        sl: 1.08000,
        tp: 1.09500,
        price_current: 1.08750,
        profit: 125.50,
        swap: -2.30,
        symbol,
        comment,
    };

    let pos = Position::from_raw(raw);
    assert_eq!(pos.ticket, 1001);
    assert_eq!(pos.time, 1700000000);
    assert_eq!(pos.position_type, OrderType::Buy);
    assert!(pos.is_buy());
    assert!(!pos.is_sell());
    assert!(pos.matches_magic(998877));
    assert!(!pos.matches_magic(123));
    assert_eq!(pos.volume, 0.5);
    assert_eq!(pos.price_open, 1.08500);
    assert_eq!(pos.stop_loss, 1.08000);
    assert_eq!(pos.take_profit, 1.09500);
    assert_eq!(pos.price_current, 1.08750);
    assert_eq!(pos.profit, 125.50);
    assert_eq!(pos.swap, -2.30);
    assert_eq!(pos.symbol, "EURUSD");
    assert_eq!(pos.comment, "cid:ord-1234");

    // Test Sell position
    raw.position_type = 1;
    let pos_sell = Position::from_raw(raw);
    assert_eq!(pos_sell.position_type, OrderType::Sell);
    assert!(pos_sell.is_sell());
    assert!(!pos_sell.is_buy());
}

#[test]
fn test_working_order_wire_decoding_and_helpers() {
    let mut symbol = [0u8; 32];
    symbol[..6].copy_from_slice(b"GBPUSD");
    let mut comment = [0u8; 32];
    comment[..17].copy_from_slice(b"cid:ord-5678:test");

    let mut raw = mt5_bridge::ffi::Mt5Order {
        ticket: 2002,
        time_setup: 1700000010,
        order_type: 2, // BuyLimit
        magic: 998877,
        volume_initial: 1.0,
        volume_current: 0.4,
        price_open: 1.08000,
        sl: 1.07500,
        tp: 1.09000,
        price_current: 1.08300,
        symbol,
        comment,
    };

    let ord = WorkingOrder::from_raw(raw);
    assert_eq!(ord.ticket, 2002);
    assert_eq!(ord.time_setup, 1700000010);
    assert_eq!(ord.order_type, OrderType::BuyLimit);
    assert!(ord.matches_magic(998877));
    assert_eq!(ord.volume_initial, 1.0);
    assert_eq!(ord.volume_current, 0.4);
    assert_eq!(ord.price_open, 1.08000);
    assert_eq!(ord.stop_loss, 1.07500);
    assert_eq!(ord.take_profit, 1.09000);
    assert_eq!(ord.price_current, 1.08300);
    assert_eq!(ord.symbol, "GBPUSD");
    assert_eq!(ord.comment, "cid:ord-5678:test");

    // Test different order types
    raw.order_type = 3;
    assert_eq!(WorkingOrder::from_raw(raw).order_type, OrderType::SellLimit);
    raw.order_type = 4;
    assert_eq!(WorkingOrder::from_raw(raw).order_type, OrderType::BuyStop);
    raw.order_type = 5;
    assert_eq!(WorkingOrder::from_raw(raw).order_type, OrderType::SellStop);
    raw.order_type = 1;
    assert_eq!(WorkingOrder::from_raw(raw).order_type, OrderType::Sell);
}

#[test]
fn test_price_tick_calculations() {
    // Basic conversions
    assert_eq!(price_to_ticks(1.08500, 0.00001), 108500);
    assert_eq!(price_to_ticks(0.0, 0.00001), 0);
    assert_eq!(price_to_ticks(100.0, 0.0), 0);
    assert_eq!(price_to_ticks(f64::NAN, 0.00001), 0);
    assert_eq!(price_to_ticks(f64::INFINITY, 0.00001), 0);
    assert_eq!(price_to_ticks(5250.25, 0.25), 21001);

    assert_eq!(ticks_to_price(108500, 0.00001, 5), 1.08500);
    assert_eq!(ticks_to_price(21001, 0.25, 2), 5250.25);

    // Stop Loss and Take Profit tick arithmetic
    // Buy SL should be below entry
    assert_eq!(calculate_sl_ticks(1.08500, 50, true, 0.00001, 5), 1.08450);
    // Buy TP should be above entry
    assert_eq!(calculate_tp_ticks(1.08500, 100, true, 0.00001, 5), 1.08600);
    // Sell SL should be above entry
    assert_eq!(calculate_sl_ticks(1.08500, 50, false, 0.00001, 5), 1.08550);
    // Sell TP should be below entry
    assert_eq!(calculate_tp_ticks(1.08500, 100, false, 0.00001, 5), 1.08400);

    // Sign of distance_ticks should not flip direction (uses abs)
    assert_eq!(calculate_sl_ticks(1.08500, -50, true, 0.00001, 5), 1.08450);
    assert_eq!(calculate_tp_ticks(1.08500, -100, true, 0.00001, 5), 1.08600);
}

#[test]
fn test_order_request_client_order_id_and_comment() {
    let req_no_cid = OrderRequest::buy("EURUSD", 0.1).comment("plain order");
    assert_eq!(req_no_cid.effective_comment(), "plain order");

    // With a client_order_id the wire comment carries a fixed-width hash token, not the raw ID.
    let req_with_cid = OrderRequest::buy("EURUSD", 0.1).client_order_id("abc-123");
    assert_eq!(
        req_with_cid.effective_comment(),
        format!("cid:{}", wire_id("abc-123"))
    );

    let req_both = OrderRequest::buy("EURUSD", 0.1)
        .client_order_id("abc-123")
        .comment("scalp");
    assert_eq!(
        req_both.effective_comment(),
        format!("cid:{}:scalp", wire_id("abc-123"))
    );

    // Comment truncation at 31 chars
    let long_cmt = "1234567890123456789012345678901234567890";
    let req_trunc = OrderRequest::buy("EURUSD", 0.1).comment(long_cmt);
    assert_eq!(req_trunc.effective_comment().len(), 31);
    assert_eq!(req_trunc.effective_comment(), &long_cmt[..31]);

    // A very long client_order_id no longer matters to the wire comment length or identity:
    // only the free-text part is ever shortened.
    let req_cid_trunc = OrderRequest::buy("EURUSD", 0.1)
        .client_order_id("very-long-client-order-id-1234567890")
        .comment("hello");
    let eff = req_cid_trunc.effective_comment();
    assert!(eff.len() <= 31);
    assert_eq!(
        eff,
        format!("cid:{}:hello", wire_id("very-long-client-order-id-1234567890"))
    );
}

#[test]
fn test_tracked_order_lifecycle_and_accounting() {
    let req = OrderRequest::buy("EURUSD", 1.0).magic(444);
    let mut tracked = TrackedOrder::new(&req, "ord-test-1");

    assert_eq!(tracked.client_order_id, "ord-test-1");
    assert_eq!(tracked.state, OrderState::Created);
    assert_eq!(tracked.requested_volume, 1.0);
    assert_eq!(tracked.filled_volume, 0.0);
    assert_eq!(tracked.remaining_volume, 1.0);
    assert_eq!(tracked.magic, 444);
    assert!(tracked.is_active());
    assert!(!tracked.is_terminal());

    tracked.mark_submitting();
    assert_eq!(tracked.state, OrderState::Submitting);
    assert!(tracked.is_active());

    // Partial fill update
    let partial_res = TradeResult {
        retcode: 10010, // TRADE_RETCODE_DONE_PARTIAL
        deal: 101,
        order: 201,
        position: 301,
        volume: 0.4,
        price: 1.0855,
    };
    tracked.update_from_trade_result(&partial_res);
    assert_eq!(tracked.state, OrderState::PartiallyFilled);
    assert_eq!(tracked.filled_volume, 0.4);
    assert!((tracked.remaining_volume - 0.6).abs() < 1e-6);
    assert_eq!(tracked.deal_ticket, 101);
    assert_eq!(tracked.order_ticket, 201);
    assert_eq!(tracked.position_ticket, 301);
    assert!(tracked.is_active());
    assert!(!tracked.is_terminal());

    // Complete fill update
    let fill_res = TradeResult {
        retcode: 10009, // TRADE_RETCODE_DONE
        deal: 102,
        order: 201,
        position: 301,
        volume: 1.0,
        price: 1.0860,
    };
    tracked.update_from_trade_result(&fill_res);
    assert_eq!(tracked.state, OrderState::Filled);
    assert_eq!(tracked.filled_volume, 1.0);
    assert_eq!(tracked.remaining_volume, 0.0);
    assert!(tracked.is_terminal());
    assert!(!tracked.is_active());

    // Mark unknown and terminal states
    let mut unknown_order = TrackedOrder::new(&req, "ord-unknown");
    unknown_order.mark_unknown("Pipe broken after write");
    assert_eq!(unknown_order.state, OrderState::Unknown);
    assert_eq!(
        unknown_order.error_message,
        Some("Pipe broken after write".to_string())
    );
    assert!(unknown_order.is_active());
    assert!(!unknown_order.is_terminal());
}

#[test]
fn test_order_manager_tracking_and_reconciliation() {
    // Permissive policy so this test can exercise the "confirmed absent -> Rejected" path in a
    // single pass. The default policy's grace period is covered in tests/failure_injection.rs.
    let policy = SafetyPolicy {
        min_absent_observations: 1,
        min_absent_secs: 0,
        ..Default::default()
    };
    let mut mgr = OrderManager::with_policy(777888, policy);
    assert_eq!(mgr.strategy_magic(), 777888);
    assert_eq!(mgr.lifecycle(), LifecycleState::Starting);

    mgr.set_lifecycle(LifecycleState::Connecting);
    assert_eq!(mgr.lifecycle(), LifecycleState::Connecting);

    // Track order directly (restart recovery simulation)
    let req = OrderRequest::buy("EURUSD", 0.5).magic(777888);
    let mut order1 = TrackedOrder::new(&req, "ord-1");
    order1.state = OrderState::Filled;
    order1.position_ticket = 5001;
    mgr.track_order(order1);

    let mut order2 = TrackedOrder::new(&req, "ord-2");
    order2.state = OrderState::Unknown;
    mgr.track_order(order2);

    let mut order3 = TrackedOrder::new(&req, "ord-3");
    order3.state = OrderState::Unknown;
    mgr.track_order(order3);

    assert_eq!(mgr.tracked_orders().len(), 3);
    assert!(mgr.get_order("ord-1").is_some());
    assert!(mgr.get_order("ord-missing").is_none());

    // Export orders
    let exported = mgr.export_orders();
    assert_eq!(exported.len(), 3);

    // Restore into fresh manager
    let mut restored_mgr = OrderManager::new(777888);
    restored_mgr.restore_orders(exported);
    assert_eq!(restored_mgr.tracked_orders().len(), 3);

    // Prepare simulated live positions and orders for reconciliation
    let mut sym_eurusd = [0u8; 32];
    sym_eurusd[..6].copy_from_slice(b"EURUSD");

    // 1) Position 5001 matches order1 (already filled)
    let pos_raw1 = mt5_bridge::ffi::Mt5Position {
        ticket: 5001,
        magic: 777888,
        volume: 0.5,
        symbol: sym_eurusd,
        ..Default::default()
    };
    let pos1 = Position::from_raw(pos_raw1);

    // 2) Position 5002 matches order2 by its wire-token comment
    let mut cmt_ord2 = [0u8; 32];
    let wire_cmt2 = format!("cid:{}", wire_id("ord-2"));
    cmt_ord2[..wire_cmt2.len()].copy_from_slice(wire_cmt2.as_bytes());
    let pos_raw2 = mt5_bridge::ffi::Mt5Position {
        ticket: 5002,
        magic: 777888,
        volume: 0.5,
        symbol: sym_eurusd,
        comment: cmt_ord2,
        ..Default::default()
    };
    let pos2 = Position::from_raw(pos_raw2);

    // 3) Foreign position 9999 owned by this magic but not in memory
    let pos_raw3 = mt5_bridge::ffi::Mt5Position {
        ticket: 9999,
        magic: 777888,
        volume: 1.0,
        symbol: sym_eurusd,
        ..Default::default()
    };
    let pos3 = Position::from_raw(pos_raw3);

    // Run reconciliation pass (empty-but-present deal history: "nothing executed in the window")
    let report = mgr.reconcile_with_deals(vec![pos1, pos2, pos3], vec![], vec![]);

    assert_eq!(mgr.lifecycle(), LifecycleState::Ready);
    assert!(report.deal_history_available);
    assert!(!report.is_clean()); // because order3 is absent and pos3 is foreign

    // order2 was Unknown -> Reconciled via live position comment match
    assert_eq!(report.reconciled_orders.len(), 1);
    assert_eq!(report.reconciled_orders[0].client_order_id, "ord-2");
    assert_eq!(report.reconciled_orders[0].state, OrderState::Reconciled);
    assert_eq!(report.reconciled_orders[0].position_ticket, 5002);

    // order3 was Unknown -> Confirmed absent -> Rejected
    assert_eq!(report.absent_orders.len(), 1);
    assert_eq!(report.absent_orders[0].client_order_id, "ord-3");
    assert_eq!(report.absent_orders[0].state, OrderState::Rejected);

    // pos3 was not tracked -> Foreign position detected
    assert_eq!(report.foreign_positions.len(), 1);
    assert_eq!(report.foreign_positions[0].ticket, 9999);
}

#[test]
fn test_stream_config_and_backpressure() {
    let cfg = StreamConfig::default();
    assert_eq!(cfg.buffer_size, 1024);
    assert_eq!(cfg.poll_interval, std::time::Duration::from_millis(50));
    assert_eq!(cfg.backpressure, BackpressurePolicy::DropLatest);

    let custom = StreamConfig {
        buffer_size: 512,
        poll_interval: std::time::Duration::from_millis(100),
        backpressure: BackpressurePolicy::Block,
    };
    assert_eq!(custom.backpressure, BackpressurePolicy::Block);
}

#[test]
fn test_all_wire_abi_layouts_and_sizes() {
    use mt5_bridge::ffi::*;

    // Compile-time & runtime ABI packing size assertions
    assert_eq!(std::mem::size_of::<Mt5SymInfo>(), 60);
    assert_eq!(std::mem::size_of::<Mt5Rate>(), 60);
    assert_eq!(std::mem::size_of::<Mt5Tick>(), 44);
    assert_eq!(std::mem::size_of::<Mt5TradeResult>(), 44);
    assert_eq!(std::mem::size_of::<Mt5Position>(), 148);
    assert_eq!(std::mem::size_of::<Mt5Order>(), 140);
}

#[test]
fn test_mt5_retcode_descriptions() {
    assert_eq!(
        mt5_retcode_description(10009),
        "TRADE_RETCODE_DONE: Request completed"
    );
    assert_eq!(
        mt5_retcode_description(10008),
        "TRADE_RETCODE_PLACED: Order placed"
    );
    assert_eq!(
        mt5_retcode_description(10010),
        "TRADE_RETCODE_DONE_PARTIAL: Only part of the request was completed"
    );
    assert_eq!(
        mt5_retcode_description(10004),
        "TRADE_RETCODE_REQUOTE: Requote"
    );
    assert_eq!(
        mt5_retcode_description(10016),
        "TRADE_RETCODE_INVALID_STOPS: Invalid stops (SL/TP) in the request"
    );
    assert_eq!(
        mt5_retcode_description(10019),
        "TRADE_RETCODE_NO_MONEY: There is not enough money to complete the request"
    );
    assert_eq!(
        mt5_retcode_description(10031),
        "TRADE_RETCODE_CONNECTION: No connection with the trade server"
    );
    assert_eq!(
        mt5_retcode_description(999999),
        "Unknown MT5 retcode"
    );
}

#[test]
fn test_mt5_error_display_formatting() {
    let err_unkn = Mt5Error::UnknownExecutionState {
        symbol: "EURUSD".to_string(),
        client_order_id: Some("cid-42".to_string()),
        description: "pipe closed".to_string(),
    };
    assert!(err_unkn.to_string().contains("status is UNKNOWN"));
    assert!(err_unkn.to_string().contains("cid-42"));

    let err_tx = Mt5Error::TransmissionFailed("write error".to_string());
    assert!(err_tx.to_string().contains("Failed to transmit"));

    let err_own = Mt5Error::OwnershipMismatch {
        ticket: 100,
        expected_magic: 555,
        actual_magic: 666,
    };
    assert!(err_own.to_string().contains("does not match expected strategy magic"));

    let err_rec = Mt5Error::ReconciliationError("mismatch".to_string());
    assert_eq!(err_rec.to_string(), "Reconciliation error: mismatch");

    let err_range = Mt5Error::InvalidTimeRange { start: 200, end: 100 };
    assert!(err_range.to_string().contains("greater than end timestamp"));
}

#[test]
fn test_bar_technical_metrics() {
    let bullish = Bar {
        time: 1700000000,
        open: 1.1000,
        high: 1.1060,
        low: 1.0990,
        close: 1.1050,
        volume: 100.0,
    };
    assert!(bullish.is_bullish());
    assert!(!bullish.is_bearish());
    assert!((bullish.mid() - 1.1025).abs() < 1e-6);
    assert!((bullish.typical_price() - ((1.1060 + 1.0990 + 1.1050) / 3.0)).abs() < 1e-6);
    assert!((bullish.range() - 0.0070).abs() < 1e-6);
    // True range compared to prev close 1.0980 (high - prev_close is 0.0080)
    assert!((bullish.true_range(1.0980) - 0.0080).abs() < 1e-6);

    let bearish = Bar {
        time: 1700000060,
        open: 1.1050,
        high: 1.1055,
        low: 1.0970,
        close: 1.0980,
        volume: 120.0,
    };
    assert!(bearish.is_bearish());
    assert!(!bearish.is_bullish());
    assert!((bearish.body() - 0.0070).abs() < 1e-6);
}

#[test]
fn test_order_type_properties() {
    assert!(OrderType::Buy.is_buy());
    assert!(!OrderType::Buy.is_sell());

    assert!(OrderType::BuyLimit.is_buy());
    assert!(!OrderType::BuyLimit.is_sell());

    assert!(OrderType::BuyStop.is_buy());
    assert!(!OrderType::BuyStop.is_sell());

    assert!(OrderType::Sell.is_sell());
    assert!(!OrderType::Sell.is_buy());

    assert!(OrderType::SellLimit.is_sell());
    assert!(!OrderType::SellLimit.is_buy());

    assert!(OrderType::SellStop.is_sell());
    assert!(!OrderType::SellStop.is_buy());
}

#[test]
fn test_symbol_info_lot_and_point_helpers() {
    assert_eq!(SymbolInfo::calculate_lot_digits(0.01), 2);
    assert_eq!(SymbolInfo::calculate_lot_digits(0.001), 3);
    assert_eq!(SymbolInfo::calculate_lot_digits(0.1), 1);
    assert_eq!(SymbolInfo::calculate_lot_digits(1.0), 0);
    assert_eq!(SymbolInfo::calculate_lot_digits(-0.01), 2);

    let sym = SymbolInfo {
        symbol: "TEST".to_string(),
        point: 0.0001,
        tick_value: 10.0,
        tick_size: 0.0001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 50.0,
        spread: 1.5,
        digits: 4,
    };
    assert_eq!(sym.lot_digits(), 2);
    assert!(sym.is_min_lot_zero_aligned());
    assert!((sym.point_value(1.0) - 10.0).abs() < 1e-6);
    assert_eq!(sym.round_price(1.23456), 1.2346);
}

#[test]
fn test_order_request_stops_validation_with_symbol() {
    let sym = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00005,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.2,
        digits: 5,
    };

    // SL not aligned to tick size (0.00005)
    let bad_sl = OrderRequest::buy("EURUSD", 0.1)
        .stop_loss(1.08003);
    assert!(bad_sl.validate_with_symbol(&sym).is_err());

    // TP not aligned to tick size (0.00005)
    let bad_tp = OrderRequest::buy("EURUSD", 0.1)
        .take_profit(1.09002);
    assert!(bad_tp.validate_with_symbol(&sym).is_err());

    // Both properly aligned
    let good_stops = OrderRequest::buy("EURUSD", 0.1)
        .stop_loss(1.08000)
        .take_profit(1.09005);
    assert!(good_stops.validate_with_symbol(&sym).is_ok());
}

#[test]
fn test_reconciliation_report_serde_and_clean() {
    let report_clean = ReconciliationReport {
        positions: vec![],
        pending_orders: vec![],
        reconciled_orders: vec![],
        absent_orders: vec![],
        foreign_positions: vec![],
        timestamp: 1700000000,
        ..Default::default()
    };
    assert!(report_clean.is_clean());

    let json = serde_json::to_string(&report_clean).expect("Failed to serialize report");
    let deserialized: ReconciliationReport = serde_json::from_str(&json).expect("Failed to deserialize report");
    assert_eq!(deserialized, report_clean);

    let state = LifecycleState::Reconciling;
    let state_json = serde_json::to_string(&state).unwrap();
    let state_deser: LifecycleState = serde_json::from_str(&state_json).unwrap();
    assert_eq!(state_deser, LifecycleState::Reconciling);
}

#[test]
fn test_order_manager_magic_and_idempotency_check() {
    let mut mgr = OrderManager::new(888999);

    // 1. Order with no magic gets strategy magic assigned
    let mut req_no_magic = OrderRequest::buy("EURUSD", 0.1);
    assert_eq!(req_no_magic.magic, None);
    let check_res = mgr.check_idempotency_and_magic(&mut req_no_magic);
    assert!(check_res.is_ok());
    assert!(check_res.unwrap().is_none());
    assert_eq!(req_no_magic.magic, Some(888999));

    // 2. Order with wrong magic is rejected
    let mut req_wrong_magic = OrderRequest::buy("EURUSD", 0.1).magic(111222);
    let check_err = mgr.check_idempotency_and_magic(&mut req_wrong_magic);
    assert!(check_err.is_err());
    match check_err.unwrap_err() {
        Mt5Error::OwnershipMismatch { expected_magic, actual_magic, .. } => {
            assert_eq!(expected_magic, 888999);
            assert_eq!(actual_magic, 111222);
        }
        other => panic!("Unexpected error: {:?}", other),
    }

    // 3. Order already tracked returns Some(tracked) without re-submitting
    let req_tracked = OrderRequest::buy("EURUSD", 0.2).magic(888999);
    let mut tracked = TrackedOrder::new(&req_tracked, "ord-already-there");
    tracked.state = OrderState::Filled;
    mgr.track_order(tracked);

    let mut req_replay = OrderRequest::buy("EURUSD", 0.2)
        .magic(888999)
        .client_order_id("ord-already-there");
    let replay_res = mgr.check_idempotency_and_magic(&mut req_replay).unwrap();
    assert!(replay_res.is_some());
    let tracked_ret = replay_res.unwrap();
    assert_eq!(tracked_ret.client_order_id, "ord-already-there");
    assert_eq!(tracked_ret.state, OrderState::Filled);
}

#[test]
fn test_order_manager_pending_order_reconciliation() {
    let mut mgr = OrderManager::new(555666);

    // Track order with Unknown state
    let req = OrderRequest::pending("EURUSD", OrderType::BuyLimit, 1.0, 1.0800).magic(555666);
    let mut tracked = TrackedOrder::new(&req, "ord-limit-1");
    tracked.state = OrderState::Unknown;
    mgr.track_order(tracked);

    // Live working pending order matches by its wire-token comment
    let mut sym_bytes = [0u8; 32];
    sym_bytes[..6].copy_from_slice(b"EURUSD");
    let mut cmt_bytes = [0u8; 32];
    let wire_cmt = format!("cid:{}", wire_id("ord-limit-1"));
    cmt_bytes[..wire_cmt.len()].copy_from_slice(wire_cmt.as_bytes());

    let live_ord_raw = mt5_bridge::ffi::Mt5Order {
        ticket: 7001,
        time_setup: 1700000000,
        order_type: 2, // BuyLimit
        magic: 555666,
        volume_initial: 1.0,
        volume_current: 0.6,
        price_open: 1.0800,
        symbol: sym_bytes,
        comment: cmt_bytes,
        ..Default::default()
    };
    let live_ord = WorkingOrder::from_raw(live_ord_raw);

    let report = mgr.reconcile_with_snapshot(vec![], vec![live_ord]);
    assert_eq!(report.reconciled_orders.len(), 1);
    let rec_ord = &report.reconciled_orders[0];
    assert_eq!(rec_ord.client_order_id, "ord-limit-1");
    assert_eq!(rec_ord.order_ticket, 7001);
    assert_eq!(rec_ord.state, OrderState::Accepted);
    assert_eq!(rec_ord.remaining_volume, 0.6);
    assert!((rec_ord.filled_volume - 0.4).abs() < 1e-6);
}

#[tokio::test]
async fn test_event_bus_tick_fanout_and_isolation() {
    let bus = EventBus::new(64);
    let mut eurusd_sub1 = bus.subscribe_ticks("EURUSD");
    let mut eurusd_sub2 = bus.subscribe_ticks("EURUSD");
    let mut gbpusd_sub = bus.subscribe_ticks("GBPUSD");

    let tick_eur = Tick {
        symbol: "EURUSD".to_string(),
        time: 1710000000,
        bid: 1.08500,
        ask: 1.08510,
        last: 1.08505,
        volume: 10,
        time_msc: 1710000000123,
        flags: 6,
    };

    bus.dispatch_tick(tick_eur.clone());

    let rec1 = eurusd_sub1.recv().await.unwrap();
    let rec2 = eurusd_sub2.recv().await.unwrap();
    assert_eq!(rec1.symbol, "EURUSD");
    assert_eq!(rec1.bid, 1.08500);
    assert_eq!(rec2.symbol, "EURUSD");
    assert_eq!(rec2.bid, 1.08500);

    // GBPUSD must not receive EURUSD tick
    assert!(gbpusd_sub.try_recv().is_err());
}

#[tokio::test]
async fn test_tick_subscription_latest_mode_auto_drop_lag() {
    let bus = EventBus::new(16);
    let rx = bus.subscribe_ticks("EURUSD");
    let mut sub = TickSubscription::new("EURUSD", StreamMode::Latest, rx);

    // Buffer capacity minimum is 64; send 100 ticks to overflow the channel
    for i in 1..=100 {
        let tick = Tick {
            symbol: "EURUSD".to_string(),
            time: 1710000000 + i,
            bid: 1.08000 + (i as f64 * 0.0001),
            ask: 1.08010 + (i as f64 * 0.0001),
            last: 1.08005 + (i as f64 * 0.0001),
            volume: i as u64,
            time_msc: (1710000000 + i) * 1000,
            flags: 6,
        };
        bus.dispatch_tick(tick);
    }

    // In Latest mode, recv() handles Lagged, updates dropped_ticks, and returns the freshest quote
    let next_tick = sub.recv().await.unwrap();
    assert!(sub.dropped_ticks() > 0);
    assert!(next_tick.bid > 1.08100);
}

#[tokio::test]
async fn test_tick_subscription_lossless_mode_checked_lag() {
    let bus = EventBus::new(16);
    let rx = bus.subscribe_ticks("EURUSD");
    let mut sub = TickSubscription::new("EURUSD", StreamMode::Lossless, rx);

    for i in 1..=100 {
        let tick = Tick {
            symbol: "EURUSD".to_string(),
            time: 1710000000 + i,
            bid: 1.08000 + (i as f64 * 0.0001),
            ask: 1.08010 + (i as f64 * 0.0001),
            last: 1.08005 + (i as f64 * 0.0001),
            volume: i as u64,
            time_msc: (1710000000 + i) * 1000,
            flags: 6,
        };
        bus.dispatch_tick(tick);
    }

    // recv_checked() detects the lag in Lossless mode
    let res = sub.recv_checked().await;
    assert!(res.is_err());
    match res.unwrap_err() {
        tokio::sync::broadcast::error::RecvError::Lagged(skipped) => {
            assert!(skipped > 0);
        }
        tokio::sync::broadcast::error::RecvError::Closed => panic!("Unexpected closed"),
    }
}

#[tokio::test]
async fn test_event_bus_trade_and_book_dispatch() {
    let bus = EventBus::new(64);
    let mut trade_sub = bus.subscribe_trade();
    let mut book_sub = bus.subscribe_book("USDJPY");

    let trade = TradeEvent {
        deal: 9901,
        order: 8801,
        position: 7701,
        time: 1710005000,
        trans_type: 1,
        order_type: OrderType::Buy,
        price: 1.0850,
        volume: 0.5,
        sl: 1.0800,
        tp: 1.0900,
        symbol: "EURUSD".to_string(),
        comment: "test deal".to_string(),
    };
    bus.dispatch_trade(trade.clone());

    let rec_trade = trade_sub.recv().await.unwrap();
    assert_eq!(rec_trade.deal, 9901);
    assert_eq!(rec_trade.symbol, "EURUSD");

    let book = BookEvent {
        symbol: "USDJPY".to_string(),
        time_msc: 1710005000123,
        is_buy: true,
        price: 155.250,
        volume: 25.0,
    };
    bus.dispatch_book(book.clone());

    let rec_book = book_sub.recv().await.unwrap();
    assert_eq!(rec_book.symbol, "USDJPY");
    assert_eq!(rec_book.price, 155.250);
}

#[test]
fn test_raw_event_wire_conversions() {
    use mt5_bridge::ffi::{Mt5BookEvent, Mt5TickEvent, Mt5TradeEvent};

    let mut sym = [0u8; 32];
    sym[..6].copy_from_slice(b"EURUSD");

    let raw_tick = Mt5TickEvent {
        symbol: sym,
        time_msc: 1710000000456,
        bid: 1.08500,
        ask: 1.08520,
        last: 1.08510,
        volume: 50,
        flags: 6,
    };
    let tick = Tick::from_event(raw_tick);
    assert_eq!(tick.symbol, "EURUSD");
    assert_eq!(tick.bid, 1.08500);
    assert_eq!(tick.ask, 1.08520);
    assert_eq!(tick.time_msc, 1710000000456);
    assert_eq!(tick.time, 1710000000);

    let mut cmt = [0u8; 32];
    cmt[..9].copy_from_slice(b"tp filled");
    let raw_trade = Mt5TradeEvent {
        deal: 12345,
        order: 67890,
        position: 11223,
        time: 1710000100,
        trans_type: 2,
        order_type: 1, // Sell
        price: 1.08520,
        volume: 1.5,
        sl: 1.08000,
        tp: 1.09000,
        symbol: sym,
        comment: cmt,
    };
    let trade = TradeEvent::from_raw(raw_trade);
    assert_eq!(trade.deal, 12345);
    assert_eq!(trade.order_type, OrderType::Sell);
    assert_eq!(trade.symbol, "EURUSD");
    assert_eq!(trade.comment, "tp filled");

    let raw_book = Mt5BookEvent {
        symbol: sym,
        time_msc: 1710000200789,
        book_type: 1, // Buy
        _pad: 0,
        price: 1.08530,
        volume: 100.0,
    };
    let book = BookEvent::from_raw(raw_book);
    assert_eq!(book.symbol, "EURUSD");
    assert!(book.is_buy);
    assert_eq!(book.price, 1.08530);
    assert_eq!(book.volume, 100.0);
}


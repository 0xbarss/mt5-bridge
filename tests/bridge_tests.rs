use mt5_bridge::*;

#[test]
fn test_timeframe_math() {
    assert_eq!(Timeframe::M1.seconds(), 60);
    assert_eq!(Timeframe::M5.seconds(), 300);
    assert_eq!(Timeframe::M15.seconds(), 900);
    assert_eq!(Timeframe::H1.seconds(), 3600);
    assert_eq!(Timeframe::D1.seconds(), 86400);
}

#[test]
fn test_round_lot_edge_cases() {
    let sym = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
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
fn test_trade_result_success_retcodes() {
    let res_done = TradeResult {
        retcode: 10009, // DONE
        deal: 1,
        order: 1,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_done.is_success());

    let res_placed = TradeResult {
        retcode: 10008, // PLACED
        deal: 0,
        order: 2,
        volume: 0.1,
        price: 1.1000,
    };
    assert!(res_placed.is_success());

    let res_partial = TradeResult {
        retcode: 10010, // DONE_PARTIAL
        deal: 3,
        order: 3,
        volume: 0.05,
        price: 1.1000,
    };
    assert!(res_partial.is_success());

    let res_err = TradeResult {
        retcode: 10019, // NO_MONEY
        deal: 0,
        order: 0,
        volume: 0.0,
        price: 0.0,
    };
    assert!(!res_err.is_success());
}

//! Tests for `mt5_bridge::types`.

#![allow(clippy::disallowed_methods)]

use super::*;
use crate::constants::{EA_DEFAULT_MAGIC, EA_DEFAULT_MAGIC_TYPED};
use crate::types::order_enums::OrderType;

#[test]
fn test_timeframe_constants() {
    assert_eq!(Timeframe::M1.to_mt5_const(), 1);
    assert_eq!(Timeframe::M15.to_mt5_const(), 15);
    assert_eq!(Timeframe::H1.to_mt5_const(), 16385);
    assert_eq!(Timeframe::D1.to_mt5_const(), 16408);
    assert_eq!(Timeframe::M1.seconds(), 60);
    assert_eq!(Timeframe::H1.seconds(), 3600);
}

#[test]
fn test_timeframe_parsing() {
    assert_eq!("m15".parse::<Timeframe>().unwrap(), Timeframe::M15);
    assert_eq!("1h".parse::<Timeframe>().unwrap(), Timeframe::H1);
    assert_eq!("D1".parse::<Timeframe>().unwrap(), Timeframe::D1);
}

#[test]
fn test_lot_rounding() {
    let sym = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.2,
        digits: 5,
        ask: 0.0,
        bid: 0.0,
    };

    assert_eq!(sym.round_lot(0.004), 0.0);
    assert_eq!(sym.round_lot(0.126), 0.13);
    assert_eq!(sym.round_lot(150.0), 100.0);
    assert!(sym.is_valid_lot(0.12));
    assert!(!sym.is_valid_lot(0.004));

    // Float noise elimination (0.29 / 0.01)
    assert_eq!(sym.round_lot(0.29), 0.29);
    assert_eq!(sym.round_lot(0.07), 0.07);

    // Tolerance for floating-point underflow slightly below min_lot
    assert_eq!(sym.round_lot(0.009999999999999998), 0.01);
    assert!(sym.is_valid_lot(0.009999999999999998));

    // Floor lot (essential for risk management so risk is not exceeded)
    assert_eq!(sym.floor_lot(0.129), 0.12);
    assert_eq!(sym.floor_lot(0.009), 0.0);

    // Ceil lot
    assert_eq!(sym.ceil_lot(0.121), 0.13);
    assert_eq!(sym.ceil_lot(0.001), 0.01);
}

#[test]
fn test_lot_rounding_min_offset_pairs() {
    // Many CFD/crypto/index pairs have min_lot that is NOT a multiple of lot_step
    // E.g., min_lot = 0.05, lot_step = 0.02 -> allowed: 0.05, 0.07, 0.09, 0.11...
    let sym = SymbolInfo {
        symbol: "CFD_INDEX".to_string(),
        point: 0.01,
        tick_value: 1.0,
        tick_size: 0.01,
        lot_step: 0.02,
        min_lot: 0.05,
        max_lot: 50.0,
        spread: 2.0,
        digits: 2,
        ask: 0.0,
        bid: 0.0,
    };

    assert!(!sym.is_min_lot_zero_aligned());
    assert_eq!(sym.lot_digits(), 2);

    // Minimum lot itself MUST be valid and round to itself
    assert!(sym.is_valid_lot(0.05));
    assert_eq!(sym.round_lot(0.05), 0.05);

    // Step increments from min_lot (0.05 + 0.02 = 0.07)
    assert!(sym.is_valid_lot(0.07));
    assert_eq!(sym.round_lot(0.07), 0.07);

    assert!(sym.is_valid_lot(0.09));
    assert_eq!(sym.round_lot(0.09), 0.09);

    // Rounding nearest to min_lot grid
    assert_eq!(sym.round_lot(0.06), 0.07);
    assert_eq!(sym.floor_lot(0.06), 0.05);
    assert_eq!(sym.ceil_lot(0.06), 0.07);

    // Invalid lot not on step grid
    assert!(!sym.is_valid_lot(0.065));

    // Sub-min lot returns 0.0
    assert_eq!(sym.round_lot(0.02), 0.0);
    assert_eq!(sym.floor_lot(0.04), 0.0);
}

#[test]
fn test_bar_calculations() {
    let bar = Bar::new(1700000000, 1.1000, 1.1050, 1.0950, 1.1020, 500.0);
    assert!((bar.mid() - 1.1000).abs() < 1e-6);
    assert!((bar.typical_price() - ((1.1050 + 1.0950 + 1.1020) / 3.0)).abs() < 1e-6);
    assert!((bar.range() - 0.0100).abs() < 1e-6);
    assert!(bar.is_bullish());
    assert!(!bar.is_bearish());
}

#[test]
fn test_account_info_helpers() {
    let acct = AccountInfo {
        balance: 10000.0,
        equity: 10500.0,
        margin: 2000.0,
        free_margin: 8500.0,
        leverage: 100.0,
        currency: "USD".to_string(),
    };
    assert_eq!(acct.profit(), 500.0);
    assert_eq!(acct.margin_level(), Some(525.0));
}

#[test]
fn test_order_request_builder() {
    let req = OrderRequest::buy("EURUSD", 0.5)
        .stop_loss(1.0850)
        .take_profit(1.0950)
        .comment("test_buy")
        .deviation(20)
        .expiration(1800000000)
        .magic(123456);

    assert_eq!(req.symbol, "EURUSD");
    assert_eq!(req.order_type, OrderType::Buy);
    assert_eq!(req.volume, 0.5);
    assert_eq!(req.stop_loss, 1.0850);
    assert_eq!(req.take_profit, 1.0950);
    assert_eq!(req.comment, "test_buy");
    assert_eq!(req.deviation, Some(20));
    assert_eq!(req.expiration, Some(1800000000));
    assert_eq!(req.magic, Some(123456));
}

#[test]
fn test_point_value_scaling() {
    // Forex: point == tick_size == 0.00001
    let forex = SymbolInfo {
        symbol: "EURUSD".to_string(),
        point: 0.00001,
        tick_value: 1.0,
        tick_size: 0.00001,
        lot_step: 0.01,
        min_lot: 0.01,
        max_lot: 100.0,
        spread: 1.0,
        digits: 5,
        ask: 0.0,
        bid: 0.0,
    };
    assert!((forex.point_value(1.0) - 1.0).abs() < 1e-6);

    // Index / CFD: point is 0.01, but tick_size is 0.25, tick_value is 12.50
    let index = SymbolInfo {
        symbol: "US500".to_string(),
        point: 0.01,
        tick_value: 12.50,
        tick_size: 0.25,
        lot_step: 0.1,
        min_lot: 0.1,
        max_lot: 100.0,
        spread: 2.0,
        digits: 2,
        ask: 0.0,
        bid: 0.0,
    };
    // 1 point (0.01) is 0.01 / 0.25 = 0.04 ticks. 0.04 * 12.50 = 0.50 per lot.
    assert!((index.point_value(1.0) - 0.50).abs() < 1e-6);
    assert!((index.point_value(2.0) - 1.00).abs() < 1e-6);
}

#[test]
fn test_price_rounding() {
    let sym = SymbolInfo {
        symbol: "US500".to_string(),
        point: 0.01,
        tick_value: 12.50,
        tick_size: 0.25,
        lot_step: 0.1,
        min_lot: 0.1,
        max_lot: 100.0,
        spread: 2.0,
        digits: 2,
        ask: 0.0,
        bid: 0.0,
    };
    assert_eq!(sym.round_price(5000.12), 5000.0);
    assert_eq!(sym.round_price(5000.13), 5000.25);
    assert_eq!(sym.round_price(5000.37), 5000.25);
    assert_eq!(sym.round_price(5000.38), 5000.50);
}

#[test]
fn test_order_request_builder_market_buy() {
    let req = OrderRequest::market_buy("EURUSD", Volume::lots(0.10))
        .stop_loss(Price(1.0800))
        .take_profit(Price(1.0900))
        .deviation(20)
        .comment("breakout")
        .client_order_id("eurusd-breakout-001")
        .build()
        .expect("valid order request");

    assert_eq!(req.symbol, "EURUSD");
    assert_eq!(req.order_type, OrderType::Buy);
    assert_eq!(req.volume, 0.10);
    assert_eq!(req.price, 0.0);
    assert_eq!(req.stop_loss, 1.0800);
    assert_eq!(req.take_profit, 1.0900);
    assert_eq!(req.comment, "breakout");
    assert_eq!(req.deviation, Some(20));
    assert_eq!(req.expiration, None);
    assert_eq!(req.magic, None);
    assert_eq!(req.client_order_id.as_deref(), Some("eurusd-breakout-001"));
}

#[test]
fn test_order_request_builder_market_sell() {
    let req = OrderRequest::market_sell("GBPUSD", Volume::lots(0.25))
        .stop_loss(Price(1.2700))
        .take_profit(Price(1.2600))
        .magic(EA_DEFAULT_MAGIC_TYPED)
        .build()
        .expect("valid order request");

    assert_eq!(req.symbol, "GBPUSD");
    assert_eq!(req.order_type, OrderType::Sell);
    assert_eq!(req.volume, 0.25);
    assert_eq!(req.stop_loss, 1.2700);
    assert_eq!(req.take_profit, 1.2600);
    assert_eq!(req.magic, Some(EA_DEFAULT_MAGIC));
}

#[test]
fn test_order_request_builder_pending() {
    let req = OrderRequestBuilder::pending(
        "EURUSD",
        OrderType::BuyLimit,
        Volume::lots(0.10),
        Price(1.0800),
    )
    .stop_loss(Price(1.0750))
    .take_profit(Price(1.0900))
    .expiration(1800000000)
    .build()
    .expect("valid pending order");

    assert_eq!(req.symbol, "EURUSD");
    assert_eq!(req.order_type, OrderType::BuyLimit);
    assert_eq!(req.volume, 0.10);
    assert_eq!(req.price, 1.0800);
    assert_eq!(req.stop_loss, 1.0750);
    assert_eq!(req.take_profit, 1.0900);
    assert_eq!(req.expiration, Some(1800000000));
}

#[test]
fn test_order_request_builder_validation_empty_symbol() {
    let err = OrderRequest::market_buy("", Volume::lots(0.10))
        .stop_loss(Price(1.0800))
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("symbol cannot be empty"));
}

#[test]
fn test_order_request_builder_validation_zero_volume() {
    let err = OrderRequest::market_buy("EURUSD", Volume::lots(0.0))
        .stop_loss(Price(1.0800))
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("volume"));
}

#[test]
fn test_order_request_builder_validation_nan_price() {
    let err = OrderRequest::market_buy("EURUSD", Volume::lots(0.10))
        .stop_loss(Price(f64::NAN))
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("stop loss"));
}

#[test]
fn test_order_request_builder_defaults() {
    let req = OrderRequest::market_buy("EURUSD", Volume::lots(0.10))
        .build()
        .expect("valid order request");

    assert_eq!(req.price, 0.0);
    assert_eq!(req.stop_loss, 0.0);
    assert_eq!(req.take_profit, 0.0);
    assert_eq!(req.comment, "");
    assert_eq!(req.deviation, None);
    assert_eq!(req.expiration, None);
    assert_eq!(req.magic, None);
    assert_eq!(req.client_order_id, None);
}

#[test]
fn test_volume_lots_constructor() {
    let v = Volume::lots(0.50);
    assert_eq!(v.into_inner(), 0.50);
    assert_eq!(v, Volume(0.50));
}

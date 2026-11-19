//! Integration tests for `PositionBook` deal attribution, lifecycle transitions,
//! weighted-average fill calculation, broker snapshot reconciliation, and position recovery.

use mt5_bridge::position_book::{DealReason, PositionBook, PositionState, PositionTransition};
use mt5_bridge::types::{OrderType, Position, TradeEvent};

#[test]
fn test_multi_symbol_position_attribution_and_net_exposure() {
    let mut book = PositionBook::new(None);

    // Open EURUSD Buy (0.50 lot @ 1.0850)
    let event_eur_1 = TradeEvent {
        deal: 101,
        order: 1001,
        position: 5001,
        time: 1700000000,
        trans_type: 1,
        order_type: OrderType::Buy,
        price: 1.08500,
        volume: 0.50,
        sl: 1.08000,
        tp: 1.09000,
        symbol: "EURUSD".to_string(),
        comment: "buy eur 1".to_string(),
        deal_entry: 0, // DealEntry::In
        deal_type: 0,
        deal_magic: 424242,
        deal_position_id: 5001,
        deal_reason: 0, // Client
        event_seq: 1,
    };
    let trans = book.apply_trade_event(&event_eur_1);
    assert!(matches!(trans, Some(PositionTransition::Opened(_))));

    // Open EURUSD Buy (0.30 lot @ 1.0860) - scale in
    let event_eur_2 = TradeEvent {
        deal: 102,
        order: 1002,
        position: 5001,
        time: 1700000001,
        trans_type: 1,
        order_type: OrderType::Buy,
        price: 1.08600,
        volume: 0.30,
        sl: 1.08000,
        tp: 1.09000,
        symbol: "EURUSD".to_string(),
        comment: "buy eur 2".to_string(),
        deal_entry: 0, // DealEntry::In
        deal_type: 0,
        deal_magic: 424242,
        deal_position_id: 5001,
        deal_reason: 0, // Client
        event_seq: 2,
    };
    let trans = book.apply_trade_event(&event_eur_2);
    assert_eq!(trans, None); // Scale-in returns None but updates position volume & price

    // Open GBPUSD Sell (1.00 lot @ 1.2700)
    let event_gbp = TradeEvent {
        deal: 201,
        order: 2001,
        position: 6001,
        time: 1700000002,
        trans_type: 1,
        order_type: OrderType::Sell,
        price: 1.27000,
        volume: 1.00,
        sl: 1.27500,
        tp: 1.26000,
        symbol: "GBPUSD".to_string(),
        comment: "sell gbp".to_string(),
        deal_entry: 0, // DealEntry::In
        deal_type: 1,
        deal_magic: 424242,
        deal_position_id: 6001,
        deal_reason: 3, // Expert
        event_seq: 3,
    };
    let trans = book.apply_trade_event(&event_gbp);
    assert!(matches!(trans, Some(PositionTransition::Opened(_))));

    // Verify open positions count
    let open_pos = book.open_positions();
    assert_eq!(open_pos.len(), 2);

    // Verify net exposure
    let net_eur_buy = book.net_quantity("EURUSD", OrderType::Buy);
    assert!((net_eur_buy - 0.80).abs() < 1e-9);

    let net_eur_sell = book.net_quantity("EURUSD", OrderType::Sell);
    assert_eq!(net_eur_sell, 0.0);

    let net_gbp_sell = book.net_quantity("GBPUSD", OrderType::Sell);
    assert!((net_gbp_sell - 1.00).abs() < 1e-9);

    assert_eq!(book.net_quantity("USDJPY", OrderType::Buy), 0.0);

    // Verify EURUSD weighted average price
    // (0.50 * 1.08500 + 0.30 * 1.08600) / 0.80 = (0.5425 + 0.3258) / 0.80 = 0.8683 / 0.80 = 1.085375
    let eur_pos = book.get(5001).expect("EURUSD position exists");
    assert!((eur_pos.volume - 0.80).abs() < 1e-9);
    assert!((eur_pos.entry_price - 1.085375).abs() < 1e-6);
    assert_eq!(eur_pos.deals.len(), 2);
}

#[test]
fn test_partial_and_full_close_with_deal_reasons() {
    let mut book = PositionBook::new(Some(100));

    // 1. Open 1.00 lot Buy
    let in_event = TradeEvent {
        deal: 301,
        order: 3001,
        position: 7001,
        time: 1700000000,
        trans_type: 1,
        order_type: OrderType::Buy,
        price: 2000.0,
        volume: 1.0,
        sl: 1980.0,
        tp: 2040.0,
        symbol: "XAUUSD".to_string(),
        comment: "gold long".to_string(),
        deal_entry: 0, // DealEntry::In
        deal_type: 0,
        deal_magic: 100,
        deal_position_id: 7001,
        deal_reason: 0, // Client
        event_seq: 1,
    };
    book.apply_trade_event(&in_event);

    // 2. Partial close 0.40 lot via Stop Loss
    let out_event_1 = TradeEvent {
        deal: 302,
        order: 3002,
        position: 7001,
        time: 1700000010,
        trans_type: 1,
        order_type: OrderType::Sell,
        price: 1990.0,
        volume: 0.40,
        sl: 0.0,
        tp: 0.0,
        symbol: "XAUUSD".to_string(),
        comment: "sl triggered".to_string(),
        deal_entry: 1, // DealEntry::Out
        deal_type: 1,
        deal_magic: 100,
        deal_position_id: 7001,
        deal_reason: 4, // StopLoss
        event_seq: 2,
    };
    let trans1 = book.apply_trade_event(&out_event_1);
    match trans1 {
        Some(PositionTransition::PartiallyClosed {
            remaining_volume,
            closed_volume,
            reason,
            ..
        }) => {
            assert!((remaining_volume - 0.60).abs() < 1e-9);
            assert!((closed_volume - 0.40).abs() < 1e-9);
            assert_eq!(reason, DealReason::StopLoss);
        }
        other => panic!("Expected PartiallyClosed, got {:?}", other),
    }

    assert_eq!(book.get(7001).unwrap().state, PositionState::Open);
    assert!((book.get(7001).unwrap().volume - 0.60).abs() < 1e-9);

    // 3. Full close remaining 0.60 lot via Take Profit
    let out_event_2 = TradeEvent {
        deal: 303,
        order: 3003,
        position: 7001,
        time: 1700000020,
        trans_type: 1,
        order_type: OrderType::Sell,
        price: 2020.0,
        volume: 0.60,
        sl: 0.0,
        tp: 0.0,
        symbol: "XAUUSD".to_string(),
        comment: "tp hit".to_string(),
        deal_entry: 1, // DealEntry::Out
        deal_type: 1,
        deal_magic: 100,
        deal_position_id: 7001,
        deal_reason: 5, // TakeProfit
        event_seq: 3,
    };
    let trans2 = book.apply_trade_event(&out_event_2);
    match trans2 {
        Some(PositionTransition::Closed {
            position_ticket,
            volume,
            price,
            reason,
            ..
        }) => {
            assert_eq!(position_ticket, 7001);
            assert!((volume - 0.60).abs() < 1e-9);
            assert_eq!(price, 2020.0);
            assert_eq!(reason, DealReason::TakeProfit);
        }
        other => panic!("Expected Closed, got {:?}", other),
    }

    let closed_pos = book.get_closed(7001).expect("Position in closed archive");
    assert_eq!(closed_pos.state, PositionState::Closed);
    assert_eq!(closed_pos.volume, 0.0);
    assert_eq!(closed_pos.deals.len(), 3);
    assert_eq!(book.open_positions().len(), 0);
    assert_eq!(book.net_quantity("XAUUSD", OrderType::Buy), 0.0);
}

#[test]
fn test_deal_reason_raw_roundtrip_and_classifications() {
    let reasons = [
        (0, DealReason::Client),
        (1, DealReason::Mobile),
        (2, DealReason::Web),
        (3, DealReason::Expert),
        (4, DealReason::StopLoss),
        (5, DealReason::TakeProfit),
        (6, DealReason::StopOut),
        (7, DealReason::Rollover),
        (8, DealReason::VMargin),
        (9, DealReason::Split),
        (10, DealReason::CorporateAction),
        (999, DealReason::Unknown(999)),
    ];

    for (raw, expected) in reasons {
        let parsed = DealReason::from_raw(raw);
        assert_eq!(parsed, expected);
        assert_eq!(parsed.as_raw(), raw);
    }

    assert!(DealReason::StopLoss.is_stop_loss());
    assert!(DealReason::TakeProfit.is_take_profit());
    assert!(DealReason::StopOut.is_stop_out());
    assert!(!DealReason::Client.is_stop_loss());
    assert!(DealReason::Client.is_client_or_expert());
    assert!(DealReason::Expert.is_client_or_expert());
}

#[test]
fn test_sl_tp_modification_transitions() {
    let mut book = PositionBook::new(None);

    let in_event = TradeEvent {
        deal: 401,
        order: 4001,
        position: 8001,
        time: 1700000000,
        trans_type: 1,
        order_type: OrderType::Buy,
        price: 1.10000,
        volume: 1.0,
        sl: 1.09000,
        tp: 1.12000,
        symbol: "EURUSD".to_string(),
        comment: "".to_string(),
        deal_entry: 0,
        deal_type: 0,
        deal_magic: 100,
        deal_position_id: 8001,
        deal_reason: 3,
        event_seq: 1,
    };
    book.apply_trade_event(&in_event);

    // Update SL and TP via non-deal position update event (deal == 0, position > 0)
    let mod_event = TradeEvent {
        deal: 0,
        order: 0,
        position: 8001,
        time: 1700000010,
        trans_type: 2,
        order_type: OrderType::Buy,
        price: 0.0,
        volume: 1.0,
        sl: 1.09500,
        tp: 1.11000,
        symbol: "EURUSD".to_string(),
        comment: "".to_string(),
        deal_entry: 0,
        deal_type: 0,
        deal_magic: 100,
        deal_position_id: 8001,
        deal_reason: 0,
        event_seq: 2,
    };
    let trans = book.apply_trade_event(&mod_event);
    assert_eq!(
        trans,
        Some(PositionTransition::Modified {
            position_ticket: 8001,
            stop_loss: 1.09500,
            take_profit: 1.11000,
        })
    );

    let pos = book.get(8001).unwrap();
    assert_eq!(pos.stop_loss, 1.09500);
    assert_eq!(pos.take_profit, 1.11000);
}

#[test]
fn test_broker_snapshot_reconciliation_complex() {
    let mut book = PositionBook::new(Some(100));

    // Internal book has position 9001 (0.50 lot) and 9002 (1.00 lot)
    let event1 = TradeEvent {
        deal: 501,
        order: 5001,
        position: 9001,
        time: 1700000000,
        trans_type: 1,
        order_type: OrderType::Buy,
        price: 1.0800,
        volume: 0.50,
        sl: 1.0700,
        tp: 1.0900,
        symbol: "EURUSD".to_string(),
        comment: "".to_string(),
        deal_entry: 0,
        deal_type: 0,
        deal_magic: 100,
        deal_position_id: 9001,
        deal_reason: 3,
        event_seq: 1,
    };
    book.apply_trade_event(&event1);

    let event2 = TradeEvent {
        deal: 502,
        order: 5002,
        position: 9002,
        time: 1700000000,
        trans_type: 1,
        order_type: OrderType::Sell,
        price: 1.2500,
        volume: 1.00,
        sl: 1.2600,
        tp: 1.2400,
        symbol: "GBPUSD".to_string(),
        comment: "".to_string(),
        deal_entry: 0,
        deal_type: 1,
        deal_magic: 100,
        deal_position_id: 9002,
        deal_reason: 3,
        event_seq: 2,
    };
    book.apply_trade_event(&event2);

    // Broker snapshot arrives:
    // - 9001 has updated SL/TP
    // - 9002 is absent (missing from broker snapshot -> transitions to Unknown)
    // - 9003 is new on broker (discovered -> Opened)
    let broker_positions = vec![
        Position {
            ticket: 9001,
            time: 1700000000,
            position_type: OrderType::Buy,
            magic: 100,
            volume: 0.50,
            price_open: 1.0800,
            stop_loss: 1.0750,   // Modified
            take_profit: 1.0950, // Modified
            price_current: 1.0820,
            swap: 0.0,
            profit: 40.0,
            symbol: "EURUSD".to_string(),
            comment: "".to_string(),
        },
        Position {
            ticket: 9003,
            time: 1700000005,
            position_type: OrderType::Buy,
            magic: 100,
            volume: 0.30,
            price_open: 150.00,
            stop_loss: 0.0,
            take_profit: 0.0,
            price_current: 150.10,
            swap: 0.0,
            profit: 20.0,
            symbol: "USDJPY".to_string(),
            comment: "external new".to_string(),
        },
    ];

    let transitions = book.apply_snapshot(&broker_positions);

    // 1. Position 9001 was modified
    let pos_9001 = book.get(9001).unwrap();
    assert_eq!(pos_9001.state, PositionState::Open);
    assert_eq!(pos_9001.stop_loss, 1.0750);
    assert_eq!(pos_9001.take_profit, 1.0950);

    // 2. Position 9002 transitioned to Unknown
    let pos_9002 = book.get(9002).unwrap();
    assert_eq!(pos_9002.state, PositionState::Unknown);

    // 3. Position 9003 was discovered and opened
    let pos_9003 = book.get(9003).unwrap();
    assert_eq!(pos_9003.state, PositionState::Open);
    assert!((pos_9003.volume - 0.30).abs() < 1e-9);
    assert_eq!(pos_9003.symbol, "USDJPY");

    assert_eq!(transitions.len(), 3);
    assert_eq!(book.open_positions().len(), 3); // 9001, 9002(Unknown), 9003
    assert!(book.has_unknown());
}

#[test]
fn test_unknown_state_and_recovery() {
    let mut book = PositionBook::new(Some(100));

    let event = TradeEvent {
        deal: 601,
        order: 6001,
        position: 9501,
        time: 1700000000,
        trans_type: 1,
        order_type: OrderType::Buy,
        price: 1.0800,
        volume: 0.50,
        sl: 0.0,
        tp: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "".to_string(),
        deal_entry: 0,
        deal_type: 0,
        deal_magic: 100,
        deal_position_id: 9501,
        deal_reason: 0,
        event_seq: 1,
    };
    book.apply_trade_event(&event);

    assert!(!book.has_unknown());

    // Mark unknown following an ambiguous close attempt
    book.mark_unknown(9501, "close timeout");
    assert!(book.has_unknown());
    assert_eq!(book.get(9501).unwrap().state, PositionState::Unknown);

    // Broker snapshot arrives confirming position is still Open
    let broker_positions = vec![Position {
        ticket: 9501,
        time: 1700000000,
        position_type: OrderType::Buy,
        magic: 100,
        volume: 0.50,
        price_open: 1.0800,
        stop_loss: 0.0,
        take_profit: 0.0,
        price_current: 1.0810,
        swap: 0.0,
        profit: 10.0,
        symbol: "EURUSD".to_string(),
        comment: "".to_string(),
    }];

    let trans = book.apply_snapshot(&broker_positions);
    assert!(!book.has_unknown());
    assert_eq!(book.get(9501).unwrap().state, PositionState::Open);
    assert!(matches!(
        trans.first(),
        Some(PositionTransition::StateChanged {
            new_state: PositionState::Open,
            ..
        })
    ));
}

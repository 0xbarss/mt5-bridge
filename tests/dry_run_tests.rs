//! Integration tests for the dry-run and replay backends (A7).
//!
//! These exercises [`DryRunBackend`] and [`ReplayBackend`] through the
//! [`TradingBackend`](crate::backend::TradingBackend) trait without a live terminal,
//! verifying that no real orders are ever placed and that synthetic fills behave
//! according to the configured fill model.

use mt5_bridge::backend::{
    DryRunBackend, DryRunConfig, FillModel, ReplayBackend, ReplayInput, ReplayOrder, TradingBackend,
};
use mt5_bridge::reconciliation::{OrderManager, SharedOrderManager};
use mt5_bridge::types::{DealEntry, Level, OrderRequest, OrderState};

fn buy_market(symbol: &str, volume: f64, cid: &str) -> OrderRequest {
    OrderRequest::buy(symbol, volume)
        .stop_loss(1.08000)
        .take_profit(1.09000)
        .client_order_id(cid)
}

fn pending_buy_limit(symbol: &str, volume: f64, price: f64, cid: &str) -> OrderRequest {
    OrderRequest::pending(
        symbol,
        mt5_bridge::types::OrderType::BuyLimit,
        volume,
        price,
    )
    .stop_loss(1.07000)
    .take_profit(1.10000)
    .client_order_id(cid)
}

#[test]
fn test_dry_run_immediate_market_fill() {
    let backend = DryRunBackend::new(DryRunConfig {
        fill_model: FillModel::Immediate,
        initial_balance: 10_000.0,
        slippage_bps: 0.0,
        default_fill_price: 1.08500,
    });

    let req = buy_market("EURUSD", 1.0, "dr-1");
    let res = backend.order_send(&req).unwrap();

    assert_eq!(res.retcode, 10009);
    assert!(res.is_filled());
    assert_eq!(res.deal, 2);
    assert_eq!(res.order, 1);
    assert_eq!(res.position, 1);
    assert_eq!(res.volume, 1.0);
    assert_eq!(res.price, 1.08500);

    // A position and a deal were created.
    let positions = backend.positions_filtered(None, None).unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].ticket, 1);
    assert_eq!(positions[0].volume, 1.0);
    assert_eq!(positions[0].price_open, 1.08500);
    assert!(positions[0].is_buy());

    let deals = backend.deals_filtered(0, i64::MAX, None, None).unwrap();
    assert_eq!(deals.len(), 1);
    assert_eq!(deals[0].entry, DealEntry::In);

    // Balance was debited by the notional.
    let acct = backend.account_info().unwrap();
    assert!((acct.balance - (10_000.0 - 1.0 * 1.08500)).abs() < 1e-6);
}

#[test]
fn test_dry_run_next_tick_pending_fill() {
    let backend = DryRunBackend::new(DryRunConfig {
        fill_model: FillModel::NextTick,
        initial_balance: 10_000.0,
        slippage_bps: 0.0,
        default_fill_price: 1.0,
    });

    // Current price 1.08500; pending BuyLimit at 1.08000 should NOT fill yet.
    let req = pending_buy_limit("EURUSD", 1.0, 1.08000, "dr-2");
    let res = backend.order_send(&req).unwrap();
    assert_eq!(res.retcode, 10008);
    assert!(res.is_placed());
    assert!(res.deal == 0);
    assert!(res.order > 0);

    // No tick yet → still pending, no position.
    assert!(backend.positions_filtered(None, None).unwrap().is_empty());
    assert_eq!(
        backend.pending_orders_filtered(None, None).unwrap().len(),
        1
    );

    // Inject a tick that trades through the limit: bid 1.07900 <= 1.08000.
    backend.tick("EURUSD", 1.07900, 1.08000);

    let positions = backend.positions_filtered(None, None).unwrap();
    assert_eq!(positions.len(), 1);
    assert!(positions[0].is_buy());
    assert!((positions[0].price_open - 1.07900).abs() < 1e-6);

    let pending = backend.pending_orders_filtered(None, None).unwrap();
    assert!(pending.is_empty());
}

#[test]
fn test_dry_run_bar_close_pending_fill() {
    let backend = DryRunBackend::new(DryRunConfig {
        fill_model: FillModel::BarClose,
        initial_balance: 10_000.0,
        slippage_bps: 0.0,
        default_fill_price: 1.0,
    });

    // Pending SellStop at 1.09000; current price 1.08500 → not filled.
    let req = OrderRequest::pending(
        "GBPUSD",
        mt5_bridge::types::OrderType::SellStop,
        2.0,
        1.09000,
    )
    .stop_loss(1.10000)
    .take_profit(1.07000)
    .client_order_id("dr-3");
    let res = backend.order_send(&req).unwrap();
    assert!(res.is_placed());
    assert!(backend.positions_filtered(None, None).unwrap().is_empty());

    // Bar closes at 1.09100, which is >= 1.09000 → SellStop triggers.
    backend.bar("GBPUSD", 1.09100);

    let positions = backend.positions_filtered(None, None).unwrap();
    assert_eq!(positions.len(), 1);
    assert!(!positions[0].is_buy());
    assert!((positions[0].price_open - 1.09100).abs() < 1e-6);
}

#[test]
fn test_dry_run_slippage_applied() {
    let backend = DryRunBackend::new(DryRunConfig {
        fill_model: FillModel::Immediate,
        initial_balance: 10_000.0,
        slippage_bps: 100.0, // 1%
        default_fill_price: 1.08500,
    });

    // Buy at 1.08500 with 1% slippage → 1.08500 * 1.01 = 1.09585.
    let res = backend
        .order_send(&buy_market("EURUSD", 1.0, "dr-4"))
        .unwrap();
    assert!((res.price - 1.09585).abs() < 1e-6);

    // Sell at 1.08500 with 1% slippage → 1.08500 * 0.99 = 1.07415.
    let req = OrderRequest::sell("EURUSD", 1.0)
        .stop_loss(1.07000)
        .take_profit(1.10000)
        .client_order_id("dr-5");
    let res = backend.order_send(&req).unwrap();
    assert!((res.price - 1.07415).abs() < 1e-6);
}

#[test]
fn test_dry_run_close_and_modify() {
    let backend = DryRunBackend::new(DryRunConfig::default());

    let res = backend
        .order_send(&buy_market("EURUSD", 1.0, "dr-6"))
        .unwrap();
    let position_ticket = res.position;
    let _deal_ticket = res.deal;

    // Modify SL/TP.
    let mod_res = backend
        .order_modify_with_magic(position_ticket, 0, Level::Set(1.08000), Level::Set(1.09000))
        .unwrap();
    assert_eq!(mod_res.retcode, 10009);

    // Close the position.
    let close_res = backend.order_close_with_magic(position_ticket, 0).unwrap();
    assert!(close_res.is_filled());
    assert_eq!(close_res.position, position_ticket);

    // Position removed, exit deal recorded.
    assert!(backend.positions_filtered(None, None).unwrap().is_empty());
    let deals = backend.deals_filtered(0, i64::MAX, None, None).unwrap();
    assert_eq!(deals.len(), 2);
    assert_eq!(deals[0].entry, DealEntry::In);
    assert_eq!(deals[1].entry, DealEntry::Out);
    assert_eq!(deals[1].position_id, position_ticket);

    // Balance credited back on close.
    let acct = backend.account_info().unwrap();
    assert!((acct.balance - 10_000.0).abs() < 1e-6);
}

#[test]
fn test_dry_run_price_query() {
    let backend = DryRunBackend::new(DryRunConfig::default());
    assert!(backend.price("EURUSD").is_none());

    backend.tick("EURUSD", 1.08000, 1.08100);
    let (bid, ask) = backend.price("EURUSD").unwrap();
    assert!((bid - 1.08000).abs() < 1e-6);
    assert!((ask - 1.08100).abs() < 1e-6);
}

#[test]
fn test_replay_backend_fills_match_recording() {
    let input = ReplayInput {
        orders: vec![
            ReplayOrder {
                client_order_id: "rep-1".to_string(),
                symbol: "EURUSD".to_string(),
                order_type: mt5_bridge::types::OrderType::Buy,
                volume: 1.0,
                price: 0.0,
                stop_loss: 1.08000,
                take_profit: 1.09000,
                filled: true,
                fill_price: 1.08500,
                fill_time: 1_700_000_000,
            },
            ReplayOrder {
                client_order_id: "rep-2".to_string(),
                symbol: "GBPUSD".to_string(),
                order_type: mt5_bridge::types::OrderType::SellLimit,
                volume: 2.0,
                price: 1.27000,
                stop_loss: 1.28000,
                take_profit: 1.25000,
                filled: false,
                fill_price: 0.0,
                fill_time: 0,
            },
        ],
    };
    let backend = ReplayBackend::new(input.clone());
    assert_eq!(backend.remaining(), 2);
    assert!(!backend.is_finished());

    // First order: filled.
    let res = backend
        .order_send(&buy_market("EURUSD", 1.0, "rep-1"))
        .unwrap();
    assert!(res.is_filled());
    assert_eq!(res.price, 1.08500);
    assert_eq!(res.volume, 1.0);
    assert_eq!(backend.remaining(), 1);

    // Second order: pending (not filled).
    let req = pending_buy_limit("GBPUSD", 2.0, 1.27000, "rep-2");
    let res = backend.order_send(&req).unwrap();
    assert!(res.is_placed());
    assert_eq!(res.price, 1.27000);
    assert_eq!(backend.remaining(), 0);
    assert!(backend.is_finished());

    // No more orders to replay.
    let err = backend
        .order_send(&buy_market("EURUSD", 1.0, "rep-3"))
        .unwrap_err();
    assert!(err.to_string().contains("no more orders"));
}

#[test]
fn test_replay_backend_state_tracking() {
    let input = ReplayInput {
        orders: vec![ReplayOrder {
            client_order_id: "rep-10".to_string(),
            symbol: "EURUSD".to_string(),
            order_type: mt5_bridge::types::OrderType::Buy,
            volume: 0.5,
            price: 0.0,
            stop_loss: 1.08000,
            take_profit: 1.09000,
            filled: true,
            fill_price: 1.08500,
            fill_time: 1_700_000_001,
        }],
    };
    let backend = ReplayBackend::new(input);
    backend
        .order_send(&buy_market("EURUSD", 0.5, "rep-10"))
        .unwrap();

    let positions = backend.positions_filtered(None, None).unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].volume, 0.5);
    assert!((positions[0].price_open - 1.08500).abs() < 1e-6);

    let deals = backend.deals_filtered(0, i64::MAX, None, None).unwrap();
    assert_eq!(deals.len(), 1);
    assert_eq!(deals[0].volume, 0.5);
    assert_eq!(deals[0].price, 1.08500);
}

#[test]
fn test_order_manager_with_dry_run_backend() {
    let manager = OrderManager::try_new(20240101).expect("valid magic");
    let shared = SharedOrderManager::new(manager);

    let backend = DryRunBackend::new(DryRunConfig {
        fill_model: FillModel::Immediate,
        initial_balance: 10_000.0,
        slippage_bps: 0.0,
        default_fill_price: 1.08500,
    });
    let arc_backend =
        std::sync::Arc::new(backend) as std::sync::Arc<dyn TradingBackend + Send + Sync>;

    let req = buy_market("EURUSD", 1.0, "om-1");
    let tracked = shared.submit_order(&arc_backend, req).unwrap();
    assert_eq!(tracked.state, OrderState::Filled);
    assert_eq!(tracked.position_ticket, 1);
    assert_eq!(tracked.deal_ticket, 2);

    // Reconcile against the dry-run backend's live state. The order is already Filled,
    // so reconciliation finds nothing unresolved and the report is settled.
    let report = shared.reconcile(&arc_backend).unwrap();
    assert!(report.is_settled());
    assert!(report.unresolved_orders.is_empty());
    assert_eq!(report.reconciled_orders.len(), 0);
}

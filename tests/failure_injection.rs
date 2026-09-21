//! End-to-end failure-injection tests for `OrderManager` / `SharedOrderManager`.
//!
//! These run against [`common::MockBroker`], a simulated broker + EA that can drop a response
//! *after* an order executed, drop a request *before* it executed, delay position visibility,
//! restart the EA, fill partially, and behave as a hedging or netting account. Unlike
//! `live_integration.rs` they need no MetaTrader terminal, so they always run in CI.
//!
//! They cover the comprehensive failure matrix across recovery, reconciliation, and concurrency. What they do **not**
//! cover is the DLL/named-pipe layer or the MQL5 EA itself; see `bridge_dll/tests/` for the
//! pipe-level harness and the README for the EA verification status.

mod common;

use common::*;
use mt5_bridge::*;
use std::sync::{Arc, Barrier, Mutex};

fn is_unknown(e: &Mt5Error) -> bool {
    matches!(e, Mt5Error::UnknownExecutionState { .. })
}

// ═════════════════════════════════════════════════════════════════════════════
// Basic execution outcomes
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn normal_market_order_is_filled_and_journaled_in_memory() {
    let broker = MockBroker::new();
    let mut mgr = manager();
    let t = mgr
        .submit_order(&broker, buy("EURUSD", 0.5, "n-1"))
        .unwrap();
    assert_eq!(t.state, OrderState::Filled);
    assert_eq!(t.filled_volume, 0.5);
    assert!(t.position_ticket > 0 && t.deal_ticket > 0);
    assert_eq!(t.wire_id, wire_id("n-1"));
    assert_eq!(broker.executions(), 1);
}

#[test]
fn broker_rejection_is_recorded_rejected() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::Reject(10019)); // no money
    let mut mgr = manager();
    let err = mgr
        .submit_order(&broker, buy("EURUSD", 0.5, "rej-1"))
        .unwrap_err();
    assert!(matches!(
        err,
        Mt5Error::OrderSendFailed { retcode: 10019, .. }
    ));
    assert_eq!(mgr.get_order("rej-1").unwrap().state, OrderState::Rejected);
    assert_eq!(broker.executions(), 0);
}

#[test]
fn transmission_failure_is_rejected_because_it_provably_never_left() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::TransmissionFailure);
    let mut mgr = manager();
    let err = mgr
        .submit_order(&broker, buy("EURUSD", 0.5, "tx-1"))
        .unwrap_err();
    assert!(matches!(err, Mt5Error::TransmissionFailed(_)));
    assert_eq!(mgr.get_order("tx-1").unwrap().state, OrderState::Rejected);
    assert!(
        mgr.unresolved_orders().is_empty(),
        "provable non-send must not block trading"
    );
}

#[test]
fn partial_fill_result_is_partially_filled() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::PartialFill(0.4));
    let mut mgr = manager();
    let t = mgr
        .submit_order(&broker, buy("EURUSD", 1.0, "pf-1"))
        .unwrap();
    assert_eq!(t.state, OrderState::PartiallyFilled);
    assert!((t.filled_volume - 0.4).abs() < 1e-12);
    assert!((t.remaining_volume - 0.6).abs() < 1e-12);
}

#[test]
fn invalid_requests_never_touch_state_or_the_broker() {
    let broker = MockBroker::new();
    let mut mgr = manager();
    for bad in [
        buy("EURUSD", 0.0, "bad-vol"),
        buy("", 0.1, "bad-sym"),
        OrderRequest::buy("EURUSD", 0.1).client_order_id(""),
        OrderRequest::buy("EURUSD", 0.1).client_order_id("   "),
        OrderRequest::buy("EURUSD", 0.1).client_order_id("has\nnewline"),
        OrderRequest::buy("EURUSD", 0.1).client_order_id("x".repeat(129)),
        OrderRequest::buy("EURUSD", 0.1).comment("cid:spoofed"),
        OrderRequest::buy("EURUSD", 0.1)
            .client_order_id("ok")
            .comment("nul\0inside"),
    ] {
        assert!(mgr.submit_order(&broker, bad).is_err());
    }
    assert!(
        mgr.tracked_orders().is_empty(),
        "validation failures must not create tracked orders"
    );
    assert_eq!(broker.send_calls(), 0);
}

// ═════════════════════════════════════════════════════════════════════════════
// Wire identity and non-ASCII safety
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn ids_sharing_a_long_prefix_get_distinct_wire_identities_and_both_execute() {
    // Under the old scheme both IDs were cut to the same 27-byte prefix "strategy-alpha-20260920-1",
    // so the EA replayed the FIRST order's execution for the second. Now they must differ.
    let a = "strategy-alpha-20260920-123456789-A";
    let b = "strategy-alpha-20260920-123456789-B";
    assert_eq!(&a[..27], &b[..27]);
    assert_ne!(
        OrderRequest::buy("EURUSD", 0.1)
            .client_order_id(a)
            .effective_comment(),
        OrderRequest::buy("EURUSD", 0.1)
            .client_order_id(b)
            .effective_comment()
    );

    let broker = MockBroker::new();
    let mut mgr = manager();
    let ta = mgr.submit_order(&broker, buy("EURUSD", 0.1, a)).unwrap();
    let tb = mgr.submit_order(&broker, buy("EURUSD", 0.2, b)).unwrap();
    assert_ne!(ta.position_ticket, tb.position_ticket);
    assert_eq!(
        broker.executions(),
        2,
        "second order must not be swallowed as a 'duplicate'"
    );
    assert!(
        (tb.filled_volume - 0.2).abs() < 1e-12,
        "second order must report its OWN fill"
    );
}

#[test]
fn free_text_comment_is_never_an_idempotency_key() {
    // Contract test for the fixed EA (`ExtractClientOrderId`): before the fix, two orders with
    // the same free-text comment and no client_order_id were treated as one.
    let broker = MockBroker::new();
    let r1 = OrderRequest::buy("EURUSD", 0.1).comment("scalp");
    let r2 = OrderRequest::buy("EURUSD", 0.1).comment("scalp");
    broker.order_send(&r1).unwrap();
    broker.order_send(&r2).unwrap();
    assert_eq!(broker.executions(), 2);
}

#[test]
fn reusing_a_client_order_id_for_a_different_request_is_an_error_not_a_silent_replay() {
    let broker = MockBroker::new();
    let mut mgr = manager();
    mgr.submit_order(&broker, buy("EURUSD", 0.1, "same-id"))
        .unwrap();
    let err = mgr
        .submit_order(&broker, buy("GBPUSD", 5.0, "same-id"))
        .unwrap_err();
    assert!(
        matches!(err, Mt5Error::ClientOrderIdConflict { .. }),
        "got {err:?}"
    );
    // identical replay is still a quiet no-op
    let again = mgr
        .submit_order(&broker, buy("EURUSD", 0.1, "same-id"))
        .unwrap();
    assert_eq!(again.state, OrderState::Filled);
    assert_eq!(broker.executions(), 1);
}

#[test]
fn wire_id_is_stable_fixed_width_and_pinned() {
    // These vectors are a compatibility contract: changing wire_id() orphans in-flight orders
    // across an upgrade. If this test fails, you changed the wire format — bump PROTOCOL_VERSION.
    for id in [
        "a",
        "abc-123",
        "ord-1",
        "strategy-alpha-20260920-123456789-A",
        "é",
        "",
    ] {
        let w = wire_id(id);
        assert_eq!(w.len(), WIRE_ID_LEN);
        assert!(w
            .bytes()
            .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b)));
        assert_eq!(w, wire_id(id), "must be deterministic");
    }
    assert_eq!(wire_id("abc-123"), PINNED_ABC_123);
    assert_eq!(wire_id("ord-1"), PINNED_ORD_1);
}
// Verified against an independent Python implementation of the documented algorithm
// (FNV-1a 64 + murmur3 fmix64, Crockford base32, MSB first).
const PINNED_ABC_123: &str = "CJ9VZ6B1N4SMR";
const PINNED_ORD_1: &str = "CFKBMGJMR8PQZ";

#[test]
fn wire_id_has_no_collisions_over_a_large_structured_id_space() {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for strat in ["alpha", "beta", "gamma-long-strategy-name"] {
        for day in 0..50 {
            for seq in 0..400 {
                let id = format!("{strat}-2026{day:04}-{seq}");
                assert!(seen.insert(wire_id(&id)), "collision at {id}");
            }
        }
    }
    assert_eq!(seen.len(), 3 * 50 * 400);
}

#[test]
fn parse_wire_id_accepts_only_well_formed_tokens() {
    let w = wire_id("x");
    assert_eq!(parse_wire_id(&format!("cid:{w}")), Some(w.as_str()));
    assert_eq!(parse_wire_id(&format!("cid:{w}:scalp")), Some(w.as_str()));
    assert_eq!(
        parse_wire_id(&format!("cid:{w}[sl 1.0850]")),
        Some(w.as_str()),
        "broker suffix"
    );
    assert_eq!(parse_wire_id("cid:tooshort"), None);
    assert_eq!(parse_wire_id("scalp"), None);
    assert_eq!(
        parse_wire_id(&format!("cid:{w}X")),
        None,
        "token must not run into more alphanumerics"
    );
    assert_eq!(parse_wire_id(&format!("xcid:{w}")), None);
    assert_eq!(parse_wire_id("cid:lowercase-not-a-token"), None);
    assert_eq!(parse_wire_id(""), None);
    assert!(comment_matches_client_order_id(&format!("cid:{w}:c"), "x"));
    assert!(!comment_matches_client_order_id(&format!("cid:{w}:c"), "y"));
}

#[test]
fn non_ascii_ids_and_comments_never_panic_at_any_truncation_boundary() {
    // Guard against slicing panics when a byte offset falls inside a multi-byte character.
    let glyphs = ["é", "€", "日", "🚀", "ß", "Ω"];
    for g in glyphs {
        for pad in 0..40 {
            let cid = format!("{}{}", "a".repeat(pad), g.repeat(12));
            let cmt = format!("{}{}", "b".repeat(pad), g.repeat(12));
            for req in [
                OrderRequest::buy("EURUSD", 0.1).client_order_id(cid.clone()),
                OrderRequest::buy("EURUSD", 0.1)
                    .client_order_id(cid.clone())
                    .comment(cmt.clone()),
                OrderRequest::buy("EURUSD", 0.1).comment(cmt.clone()),
            ] {
                let eff = req.effective_comment(); // must not panic
                assert!(eff.len() <= MT5_COMMENT_MAX_BYTES);
                assert!(std::str::from_utf8(eff.as_bytes()).is_ok());
            }
        }
    }
    // and through the manager, end to end
    let broker = MockBroker::new();
    let mut mgr = manager();
    let t = mgr
        .submit_order(
            &broker,
            OrderRequest::buy("EURUSD", 0.1)
                .client_order_id("ID-日本語-🚀-é")
                .comment("コメント日本語コメント日本語"),
        )
        .unwrap();
    assert_eq!(t.state, OrderState::Filled);
    assert_eq!(truncate_utf8("日本語", 4), "日");
    assert_eq!(truncate_utf8("日本語", 2), "");
    assert_eq!(truncate_utf8("abc", 10), "abc");
}

#[test]
fn generated_client_order_ids_are_unique_across_threads() {
    use std::collections::HashSet;
    let handles: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                (0..2000)
                    .map(|_| OrderManager::generate_client_order_id())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut all = HashSet::new();
    let mut total = 0;
    for h in handles {
        for id in h.join().unwrap() {
            total += 1;
            all.insert(id);
        }
    }
    assert_eq!(all.len(), total, "generated IDs must never repeat");
}

#[test]
fn tracked_order_state_exported_by_older_versions_still_deserializes() {
    // No wire_id / requested_* / deal_tickets / absent_* / mismatches fields.
    let old = r#"{
        "client_order_id":"legacy-1","symbol":"EURUSD","order_type":"Buy","requested_volume":0.5,
        "filled_volume":0.0,"remaining_volume":0.5,"average_price":0.0,"order_ticket":0,
        "deal_ticket":0,"position_ticket":0,"state":"Unknown","magic":424242,
        "created_at":1700000000,"updated_at":1700000000,"error_message":null}"#;
    let t: TrackedOrder = serde_json::from_str(old).expect("legacy state must load");
    assert_eq!(t.state, OrderState::Unknown);
    assert!(t.wire_id.is_empty() && t.deal_tickets.is_empty() && t.absent_observations == 0);
    let mut mgr = manager();
    mgr.restore_orders(vec![t]);
    assert_eq!(
        mgr.get_order("legacy-1").unwrap().wire_id,
        wire_id("legacy-1"),
        "wire id backfilled"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Lost responses → Unknown → reconciliation
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn response_lost_after_execution_is_unknown_then_reconciled_from_the_live_position() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    let mut mgr = manager();

    let err = mgr
        .submit_order(&broker, buy("EURUSD", 0.5, "lost-1"))
        .unwrap_err();
    assert!(is_unknown(&err));
    assert_eq!(mgr.get_order("lost-1").unwrap().state, OrderState::Unknown);
    assert_eq!(mgr.lifecycle(), LifecycleState::Degraded);
    assert_eq!(broker.executions(), 1, "the order really did execute");

    let report = mgr.reconcile(&broker).unwrap();
    let t = mgr.get_order("lost-1").unwrap();
    assert_eq!(t.state, OrderState::Reconciled);
    assert!(t.position_ticket > 0 && (t.filled_volume - 0.5).abs() < 1e-12);
    assert_eq!(report.reconciled_orders.len(), 1);
    assert!(report.is_clean() && report.is_settled());
    assert_eq!(mgr.lifecycle(), LifecycleState::Ready);
    assert_eq!(broker.executions(), 1, "reconciliation must never re-send");
}

#[test]
fn response_lost_before_execution_resolves_to_rejected_only_after_confirmed_absence() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::LoseBeforeExecution);
    let mut mgr = manager();

    assert!(is_unknown(
        &mgr.submit_order(&broker, buy("EURUSD", 0.5, "lost-2"))
            .unwrap_err()
    ));
    assert_eq!(broker.executions(), 0);

    let report = mgr.reconcile(&broker).unwrap();
    assert_eq!(report.absent_orders.len(), 1);
    assert_eq!(mgr.get_order("lost-2").unwrap().state, OrderState::Rejected);
    assert!(mgr.unresolved_orders().is_empty());
}

#[test]
fn delayed_broker_visibility_keeps_the_order_unknown_instead_of_misclassifying_it() {
    // The position exists but is not yet visible. Default grace policy must wait.
    let broker = MockBroker::new();
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    broker.set_visibility_delay(2);
    let mut mgr = OrderManager::with_policy(
        MAGIC,
        SafetyPolicy {
            min_absent_observations: 4,
            min_absent_secs: 0,
            ..Default::default()
        },
    );
    assert!(is_unknown(
        &mgr.submit_order(&broker, buy("EURUSD", 0.5, "slow-1"))
            .unwrap_err()
    ));

    for _ in 0..2 {
        let r = mgr.reconcile(&broker).unwrap();
        assert_eq!(
            r.unresolved_orders.len(),
            1,
            "not visible yet → still Unknown, not Rejected"
        );
        assert_eq!(mgr.get_order("slow-1").unwrap().state, OrderState::Unknown);
    }
    let r = mgr.reconcile(&broker).unwrap();
    assert_eq!(r.reconciled_orders.len(), 1);
    assert_eq!(
        mgr.get_order("slow-1").unwrap().state,
        OrderState::Reconciled
    );
    assert_eq!(
        mgr.get_order("slow-1").unwrap().absent_observations,
        0,
        "absence counter reset when found"
    );
}

#[test]
fn single_shot_policy_misclassifies_delayed_visibility_which_is_why_it_is_not_the_default() {
    // Documents the hazard: with a one-snapshot policy the same scenario yields Rejected for an
    // order that in fact exists.
    let broker = MockBroker::new();
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    broker.set_visibility_delay(2);
    let mut mgr = manager(); // instant_policy: 1 observation, 0 s
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "slow-2"));
    mgr.reconcile(&broker).unwrap();
    assert_eq!(mgr.get_order("slow-2").unwrap().state, OrderState::Rejected);
    assert_eq!(
        broker.executions(),
        1,
        "…while a live position exists on the broker"
    );
}

#[test]
fn default_policy_needs_multiple_observations_and_elapsed_time_before_rejecting() {
    let mut mgr = OrderManager::new(MAGIC); // SafetyPolicy::default(): 3 obs, 30 s, history required
    let broker = MockBroker::new();
    broker.push_fault(Fault::LoseBeforeExecution);
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "grace-1"));
    let t0 = chrono::Utc::now().timestamp();
    let snap = || BrokerSnapshot {
        deals: Some(vec![]),
        ..Default::default()
    };

    // Three quick observations are not enough without elapsed time…
    for i in 0..3 {
        let r = mgr.reconcile_snapshot_at(snap(), t0 + i);
        assert_eq!(r.unresolved_orders.len(), 1);
    }
    assert_eq!(mgr.get_order("grace-1").unwrap().state, OrderState::Unknown);
    // …and elapsed time alone is not enough without observations (this is the 4th, 31 s in).
    let r = mgr.reconcile_snapshot_at(snap(), t0 + 31);
    assert_eq!(r.absent_orders.len(), 1);
    assert_eq!(
        mgr.get_order("grace-1").unwrap().state,
        OrderState::Rejected
    );
}

#[test]
fn absence_without_deal_history_never_rejects_under_default_policy() {
    let mut mgr = OrderManager::new(MAGIC);
    let broker = MockBroker::new();
    broker.push_fault(Fault::LoseBeforeExecution);
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "nohist-1"));
    let t0 = chrono::Utc::now().timestamp();
    for i in 0..10 {
        let r = mgr.reconcile_snapshot_at(BrokerSnapshot::default(), t0 + i * 60);
        assert!(!r.deal_history_available);
        assert_eq!(r.unresolved_orders.len(), 1);
    }
    assert_eq!(
        mgr.get_order("nohist-1").unwrap().state,
        OrderState::Unknown
    );
}

#[test]
fn backend_without_history_support_leaves_absent_orders_unknown() {
    let broker = MockBroker::new();
    broker.set_history_supported(false);
    broker.push_fault(Fault::LoseBeforeExecution);
    let mut mgr = manager();
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "nohist-2"));
    let r = mgr.reconcile(&broker).unwrap();
    assert!(!r.deal_history_available);
    assert_eq!(
        mgr.get_order("nohist-2").unwrap().state,
        OrderState::Unknown
    );
    // Explicit opt-out for users of history-less setups:
    mgr.set_policy(SafetyPolicy {
        require_deal_history: false,
        ..instant_policy()
    });
    mgr.reconcile(&broker).unwrap();
    assert_eq!(
        mgr.get_order("nohist-2").unwrap().state,
        OrderState::Rejected
    );
}

#[test]
fn absence_counter_resets_when_the_order_is_seen_and_survives_persistence() {
    let mut mgr = OrderManager::new(MAGIC);
    let broker = MockBroker::new();
    broker.push_fault(Fault::LoseBeforeExecution);
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "ctr-1"));
    let t0 = chrono::Utc::now().timestamp();
    mgr.reconcile_snapshot_at(
        BrokerSnapshot {
            deals: Some(vec![]),
            ..Default::default()
        },
        t0,
    );
    assert_eq!(mgr.get_order("ctr-1").unwrap().absent_observations, 1);

    let mut restored = OrderManager::new(MAGIC);
    restored.restore_orders(mgr.export_orders());
    let o = restored.get_order("ctr-1").unwrap();
    assert_eq!((o.absent_observations, o.first_absent_at), (1, Some(t0)));
}

// ═════════════════════════════════════════════════════════════════════════════
// Deal-history reconstruction
// ═════════════════════════════════════════════════════════════════════════════

fn entry_deal(ticket: u64, order: u64, position_id: u64, cid: &str, vol: f64, price: f64) -> Deal {
    Deal {
        ticket,
        order,
        position_id,
        time: chrono::Utc::now().timestamp(),
        direction: OrderType::Buy,
        entry: DealEntry::In,
        magic: MAGIC,
        volume: vol,
        price,
        commission: 0.0,
        swap: 0.0,
        profit: 0.0,
        symbol: "EURUSD".to_string(),
        comment: format!("cid:{}", wire_id(cid)),
    }
}

/// Manager holding one Unknown BUY EURUSD order whose request never reached the broker.
fn manager_with_unknown(cid: &str, vol: f64) -> (MockBroker, OrderManager) {
    let broker = MockBroker::new();
    broker.push_fault(Fault::LoseBeforeExecution);
    let mut mgr = manager();
    let _ = mgr.submit_order(&broker, buy("EURUSD", vol, cid));
    assert_eq!(mgr.get_order(cid).unwrap().state, OrderState::Unknown);
    (broker, mgr)
}

#[test]
fn two_partial_fills_are_reconstructed_from_deals_even_after_the_position_closed() {
    let (broker, mut mgr) = manager_with_unknown("split-1", 1.0);
    // Filled as 0.4 + 0.6, then the position was closed (SL) before we ever looked.
    broker.inject_deal(entry_deal(9001, 8001, 7001, "split-1", 0.4, 1.0850));
    broker.inject_deal(entry_deal(9002, 8001, 7001, "split-1", 0.6, 1.0860));
    let mut out = entry_deal(9003, 8002, 7001, "[sl 1.0800]", 1.0, 1.0800);
    out.entry = DealEntry::Out;
    out.direction = OrderType::Sell;
    broker.inject_deal(out);

    let report = mgr.reconcile(&broker).unwrap();
    let t = mgr.get_order("split-1").unwrap();
    assert_eq!(
        t.state,
        OrderState::Reconciled,
        "mismatches: {:?}",
        t.mismatches
    );
    assert!((t.filled_volume - 1.0).abs() < 1e-12);
    assert!(
        (t.average_price - (0.4 * 1.0850 + 0.6 * 1.0860)).abs() < 1e-12,
        "volume-weighted"
    );
    assert_eq!(
        t.deal_tickets,
        vec![9001, 9002],
        "the closing deal must not be attributed"
    );
    assert_eq!(t.position_ticket, 7001);
    assert!(report.deal_history_available && report.is_clean());
    assert!(
        broker.all_positions().is_empty(),
        "no live position was needed"
    );
}

#[test]
fn deals_covering_only_part_of_the_request_are_flagged_as_a_volume_mismatch() {
    let (broker, mut mgr) = manager_with_unknown("short-1", 1.0);
    broker.inject_deal(entry_deal(9101, 8101, 7101, "short-1", 0.4, 1.0850));
    let report = mgr.reconcile(&broker).unwrap();
    let t = mgr.get_order("short-1").unwrap();
    assert_eq!(t.state, OrderState::Mismatched);
    assert!(t
        .mismatches
        .iter()
        .any(|m| m.kind == MismatchKind::Volume && m.actual == "0.4"));
    assert!(
        (t.filled_volume - 0.4).abs() < 1e-12,
        "real exposure is still recorded"
    );
    assert_eq!(report.mismatched_orders.len(), 1);
    assert!(!report.is_clean());
}

#[test]
fn partially_filled_limit_order_with_a_working_remainder() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::LoseBeforeExecution);
    let mut mgr = manager();
    let req =
        OrderRequest::pending("EURUSD", OrderType::BuyLimit, 1.0, 1.0800).client_order_id("lim-1");
    let _ = mgr.submit_order(&broker, req);

    broker.inject_deal(entry_deal(9201, 8201, 7201, "lim-1", 0.4, 1.0800));
    broker.inject_pending(WorkingOrder {
        ticket: 8201,
        time_setup: chrono::Utc::now().timestamp(),
        order_type: OrderType::BuyLimit,
        magic: MAGIC,
        volume_initial: 1.0,
        volume_current: 0.6,
        price_open: 1.0800,
        stop_loss: 0.0,
        take_profit: 0.0,
        price_current: 1.0805,
        symbol: "EURUSD".to_string(),
        comment: format!("cid:{}", wire_id("lim-1")),
    });
    mgr.reconcile(&broker).unwrap();
    let t = mgr.get_order("lim-1").unwrap();
    assert_eq!(
        t.state,
        OrderState::PartiallyFilled,
        "mismatches: {:?}",
        t.mismatches
    );
    assert!((t.filled_volume - 0.4).abs() < 1e-12 && (t.remaining_volume - 0.6).abs() < 1e-12);
    assert_eq!(t.order_ticket, 8201);
}

#[test]
fn netting_account_fill_is_attributed_by_deals_not_the_merged_position_volume() {
    let broker = MockBroker::new();
    broker.set_netting(true);
    broker.inject_position(Position {
        ticket: 77,
        time: 0,
        position_type: OrderType::Buy,
        magic: MAGIC,
        volume: 0.8,
        price_open: 1.0700,
        stop_loss: 0.0,
        take_profit: 0.0,
        price_current: 1.0850,
        profit: 0.0,
        swap: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "pre-existing".to_string(),
    });
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    let mut mgr = manager();
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "net-1"));
    assert_eq!(broker.all_positions().len(), 1);
    assert!(
        (broker.all_positions()[0].volume - 1.3).abs() < 1e-12,
        "merged into ticket 77"
    );

    let report = mgr.reconcile(&broker).unwrap();
    let t = mgr.get_order("net-1").unwrap();
    assert_eq!(
        t.state,
        OrderState::Reconciled,
        "mismatches: {:?}",
        t.mismatches
    );
    assert!(
        (t.filled_volume - 0.5).abs() < 1e-12,
        "own fill, not the 1.3 lot aggregate"
    );
    assert_eq!(t.position_ticket, 77);
    assert!(report.foreign_positions.is_empty());
}

#[test]
fn netting_without_deal_history_cannot_attribute_the_fill_and_stays_unknown() {
    // In a netting account the merged position keeps its ORIGINAL comment, so without deal
    // history there is no evidence tying it to this order. The manager must not guess: the order
    // stays Unknown (and keeps blocking) rather than being "reconciled" against someone else's
    // 0.8 lot position or rejected while 0.5 lots exist.
    let broker = MockBroker::new();
    broker.set_netting(true);
    broker.set_history_supported(false);
    broker.inject_position(Position {
        ticket: 77,
        time: 0,
        position_type: OrderType::Buy,
        magic: MAGIC,
        volume: 0.8,
        price_open: 1.07,
        stop_loss: 0.0,
        take_profit: 0.0,
        price_current: 1.08,
        profit: 0.0,
        swap: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "pre-existing".to_string(),
    });
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    let mut mgr = manager();
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "net-2"));
    let report = mgr.reconcile(&broker).unwrap();
    assert_eq!(mgr.get_order("net-2").unwrap().state, OrderState::Unknown);
    assert_eq!(report.unresolved_orders.len(), 1);
    assert!(!report.deal_history_available);
    assert_eq!(mgr.lifecycle(), LifecycleState::Degraded);
}

#[test]
fn hedging_selects_the_correct_ticket_among_same_symbol_positions() {
    let broker = MockBroker::new(); // hedging
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    let mut mgr = OrderManager::with_policy(
        MAGIC,
        SafetyPolicy {
            unknown_block_scope: UnknownBlockScope::Disabled,
            ..instant_policy()
        },
    );
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.1, "hedge-A"));
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.2, "hedge-B"));
    // plus an untracked position on the same symbol/magic
    broker.inject_position(Position {
        ticket: 5555,
        time: 0,
        position_type: OrderType::Buy,
        magic: MAGIC,
        volume: 0.3,
        price_open: 1.08,
        stop_loss: 0.0,
        take_profit: 0.0,
        price_current: 1.08,
        profit: 0.0,
        swap: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "manual".to_string(),
    });

    // Use history-less mode so position matching (not deals) is what's exercised.
    broker.set_history_supported(false);
    let report = mgr.reconcile(&broker).unwrap();
    let a = mgr.get_order("hedge-A").unwrap();
    let b = mgr.get_order("hedge-B").unwrap();
    assert_eq!(
        (a.state, b.state),
        (OrderState::Reconciled, OrderState::Reconciled)
    );
    assert_ne!(a.position_ticket, b.position_ticket);
    assert!((a.filled_volume - 0.1).abs() < 1e-12 && (b.filled_volume - 0.2).abs() < 1e-12);
    assert_eq!(report.foreign_positions.len(), 1);
    assert_eq!(report.foreign_positions[0].ticket, 5555);
}

#[test]
fn positions_of_other_strategies_are_never_touched_or_claimed() {
    let broker = MockBroker::new();
    broker.inject_position(Position {
        ticket: 6001,
        time: 0,
        position_type: OrderType::Buy,
        magic: 999,
        volume: 1.0,
        price_open: 1.08,
        stop_loss: 0.0,
        take_profit: 0.0,
        price_current: 1.08,
        profit: 0.0,
        swap: 0.0,
        symbol: "EURUSD".to_string(),
        comment: "other strategy".to_string(),
    });
    let mut mgr = manager();
    let report = mgr.reconcile(&broker).unwrap();
    assert!(report.positions.is_empty() && report.foreign_positions.is_empty());
    let err = mgr.close_position(&broker, 6001).unwrap_err();
    assert!(matches!(err, Mt5Error::OwnershipMismatch { .. }));
    assert_eq!(broker.all_positions().len(), 1, "still open");
}

// ═════════════════════════════════════════════════════════════════════════════
// Attribute comparison
// ═════════════════════════════════════════════════════════════════════════════

fn unknown_order_with_protection() -> OrderManager {
    let mut mgr = manager();
    let mut t = TrackedOrder::new(
        &OrderRequest::buy("EURUSD", 1.0)
            .stop_loss(1.0800)
            .take_profit(1.0900)
            .magic(MAGIC),
        "attr-1",
    );
    t.mark_unknown("test");
    mgr.track_order(t);
    mgr
}

fn matching_position() -> Position {
    Position {
        ticket: 4242,
        time: 0,
        position_type: OrderType::Buy,
        magic: MAGIC,
        volume: 1.0,
        price_open: 1.0850,
        stop_loss: 1.0800,
        take_profit: 1.0900,
        price_current: 1.0851,
        profit: 0.0,
        swap: 0.0,
        symbol: "EURUSD".to_string(),
        comment: format!("cid:{}", wire_id("attr-1")),
    }
}

#[test]
fn a_position_that_matches_every_attribute_is_reconciled_cleanly() {
    let mut mgr = unknown_order_with_protection();
    let r = mgr.reconcile_with_deals(vec![matching_position()], vec![], vec![]);
    assert_eq!(
        mgr.get_order("attr-1").unwrap().state,
        OrderState::Reconciled
    );
    assert!(r.is_clean() && r.mismatched_orders.is_empty());
    assert_eq!(mgr.lifecycle(), LifecycleState::Ready);
}

#[test]
fn every_attribute_disagreement_is_detected_and_reported() {
    type Case = (&'static str, Box<dyn Fn(&mut Position)>, MismatchKind);
    let cases: Vec<Case> = vec![
        (
            "direction",
            Box::new(|p| p.position_type = OrderType::Sell),
            MismatchKind::Direction,
        ),
        ("volume", Box::new(|p| p.volume = 0.5), MismatchKind::Volume),
        (
            "stop loss removed",
            Box::new(|p| p.stop_loss = 0.0),
            MismatchKind::StopLoss,
        ),
        (
            "take profit moved",
            Box::new(|p| p.take_profit = 1.1000),
            MismatchKind::TakeProfit,
        ),
        (
            "symbol",
            Box::new(|p| p.symbol = "GBPUSD".into()),
            MismatchKind::Symbol,
        ),
        ("magic", Box::new(|p| p.magic = 1), MismatchKind::Magic),
    ];
    for (name, mutate, kind) in cases {
        let mut mgr = unknown_order_with_protection();
        let mut pos = matching_position();
        mutate(&mut pos);
        let r = mgr.reconcile_with_deals(vec![pos], vec![], vec![]);
        let t = mgr.get_order("attr-1").unwrap();
        assert_eq!(t.state, OrderState::Mismatched, "{name}");
        assert!(
            t.mismatches.iter().any(|m| m.kind == kind),
            "{name}: {:?}",
            t.mismatches
        );
        assert_eq!(r.mismatched_orders.len(), 1, "{name}");
        assert!(r.reconciled_orders.is_empty() && !r.is_clean(), "{name}");
        assert_eq!(mgr.lifecycle(), LifecycleState::Degraded, "{name}");
        assert_eq!(
            t.position_ticket, 4242,
            "{name}: exposure is real and must stay linked"
        );
    }
}

#[test]
fn mismatched_pending_order_type_price_and_volume_are_detected() {
    let mut mgr = manager();
    let mut t = TrackedOrder::new(
        &OrderRequest::pending("EURUSD", OrderType::BuyLimit, 1.0, 1.0800).magic(MAGIC),
        "pend-1",
    );
    t.mark_unknown("test");
    mgr.track_order(t);
    let ord = WorkingOrder {
        ticket: 3001,
        time_setup: 0,
        order_type: OrderType::SellStop,
        magic: MAGIC,
        volume_initial: 2.0,
        volume_current: 2.0,
        price_open: 1.0900,
        stop_loss: 0.0,
        take_profit: 0.0,
        price_current: 1.0,
        symbol: "EURUSD".into(),
        comment: format!("cid:{}", wire_id("pend-1")),
    };
    mgr.reconcile_with_deals(vec![], vec![ord], vec![]);
    let kinds: Vec<_> = mgr
        .get_order("pend-1")
        .unwrap()
        .mismatches
        .iter()
        .map(|m| m.kind)
        .collect();
    for k in [
        MismatchKind::OrderType,
        MismatchKind::Direction,
        MismatchKind::Volume,
        MismatchKind::Price,
    ] {
        assert!(kinds.contains(&k), "missing {k:?} in {kinds:?}");
    }
}

#[test]
fn partial_position_from_a_lost_response_shows_up_as_a_volume_mismatch() {
    let broker = MockBroker::new();
    broker.set_history_supported(false); // force the position path
    broker.push_fault(Fault::PartialFillThenLoseResponse(0.4));
    let mut mgr = manager();
    let _ = mgr.submit_order(&broker, buy("EURUSD", 1.0, "pp-1"));
    mgr.reconcile(&broker).unwrap();
    let t = mgr.get_order("pp-1").unwrap();
    assert_eq!(t.state, OrderState::Mismatched);
    assert!(t.mismatches.iter().any(|m| m.kind == MismatchKind::Volume));
}

#[test]
fn operator_can_acknowledge_a_mismatch_and_only_a_mismatch() {
    let mut mgr = unknown_order_with_protection();
    let mut pos = matching_position();
    pos.volume = 0.5;
    mgr.reconcile_with_deals(vec![pos], vec![], vec![]);
    let t = mgr.acknowledge_mismatch("attr-1").unwrap();
    assert_eq!(t.state, OrderState::Reconciled);
    assert!(t.mismatches.is_empty());
    assert!(
        mgr.acknowledge_mismatch("attr-1").is_err(),
        "no longer Mismatched"
    );
    assert!(mgr.acknowledge_mismatch("nope").is_err());
}

#[test]
fn deliberate_sl_tp_modification_updates_the_expectation() {
    let broker = MockBroker::new();
    let mut mgr = manager();
    let t = mgr
        .submit_order(&broker, buy("EURUSD", 0.5, "mod-1"))
        .unwrap();
    mgr.modify_order(&broker, t.position_ticket, 1.0700, 1.1000)
        .unwrap();
    let o = mgr.get_order("mod-1").unwrap();
    assert_eq!(
        (o.requested_stop_loss, o.requested_take_profit),
        (1.0700, 1.1000)
    );
}

#[test]
fn an_ambiguous_match_is_never_guessed() {
    let mut mgr = unknown_order_with_protection();
    let mut p1 = matching_position();
    let mut p2 = matching_position();
    p1.ticket = 1;
    p2.ticket = 2; // two live positions both carrying attr-1's token
    let r = mgr.reconcile_with_deals(vec![p1, p2], vec![], vec![]);
    assert_eq!(mgr.get_order("attr-1").unwrap().state, OrderState::Unknown);
    assert_eq!(r.unresolved_orders.len(), 1);
}

// ═════════════════════════════════════════════════════════════════════════════
// Unknown blocks new submissions; explicit retry
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn naive_retry_with_a_new_id_after_an_unknown_that_actually_executed_is_blocked() {
    // The headline hazard: A executed but its response was lost; the caller "retries" as B.
    let broker = MockBroker::new();
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    let mut mgr = manager();
    assert!(is_unknown(
        &mgr.submit_order(&broker, buy("EURUSD", 0.5, "A"))
            .unwrap_err()
    ));

    let err = mgr
        .submit_order(&broker, buy("EURUSD", 0.5, "B"))
        .unwrap_err();
    match err {
        Mt5Error::UnresolvedUnknownOrders {
            client_order_ids, ..
        } => assert_eq!(client_order_ids, vec!["A"]),
        other => panic!("expected UnresolvedUnknownOrders, got {other:?}"),
    }
    assert_eq!(broker.executions(), 1, "exposure must not double");
    assert_eq!(
        broker.send_calls(),
        1,
        "the blocked request must not even reach the broker"
    );
    assert!(mgr.get_order("B").is_none());

    // The same-ID replay stays a harmless no-op while blocked.
    assert_eq!(
        mgr.submit_order(&broker, buy("EURUSD", 0.5, "A"))
            .unwrap()
            .state,
        OrderState::Unknown
    );

    // After reconciliation resolves A, trading resumes.
    mgr.reconcile(&broker).unwrap();
    mgr.submit_order(&broker, buy("EURUSD", 0.25, "B")).unwrap();
    assert_eq!(broker.executions(), 2);
}

#[test]
fn block_scope_strategy_symbol_and_disabled() {
    for (scope, other_symbol_allowed, same_symbol_allowed) in [
        (UnknownBlockScope::Strategy, false, false),
        (UnknownBlockScope::Symbol, true, false),
        (UnknownBlockScope::Disabled, true, true),
    ] {
        let broker = MockBroker::new();
        broker.push_fault(Fault::LoseBeforeExecution);
        let mut mgr = OrderManager::with_policy(
            MAGIC,
            SafetyPolicy {
                unknown_block_scope: scope,
                ..instant_policy()
            },
        );
        let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "U"));
        assert_eq!(
            mgr.submit_order(&broker, buy("GBPUSD", 0.1, "other"))
                .is_ok(),
            other_symbol_allowed,
            "{scope:?} other symbol"
        );
        assert_eq!(
            mgr.submit_order(&broker, buy("EURUSD", 0.1, "same"))
                .is_ok(),
            same_symbol_allowed,
            "{scope:?} same symbol"
        );
    }
}

#[test]
fn retry_order_is_refused_until_the_original_is_definitively_not_placed() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::LoseBeforeExecution);
    let mut mgr = manager();
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "R"));

    // Unknown → refused
    let err = mgr
        .retry_order(&broker, "R", OrderRequest::buy("EURUSD", 0.5))
        .unwrap_err();
    assert!(matches!(err, Mt5Error::RetryNotAllowed { .. }), "{err:?}");

    // reconcile confirms absence → Rejected → retry allowed, with a derived traceable ID
    mgr.reconcile(&broker).unwrap();
    assert_eq!(mgr.get_order("R").unwrap().state, OrderState::Rejected);
    let retried = mgr
        .retry_order(&broker, "R", OrderRequest::buy("EURUSD", 0.5))
        .unwrap();
    assert_eq!(retried.client_order_id, "R-r1");
    assert_eq!(retried.state, OrderState::Filled);
    assert_eq!(broker.executions(), 1);

    // a filled order must not be "retried" into double exposure
    let err = mgr
        .retry_order(&broker, "R-r1", OrderRequest::buy("EURUSD", 0.5))
        .unwrap_err();
    assert!(matches!(err, Mt5Error::RetryNotAllowed { .. }));
    // unknown previous id / changed intent
    assert!(mgr
        .retry_order(&broker, "ghost", OrderRequest::buy("EURUSD", 0.5))
        .is_err());
}

#[test]
fn retry_order_must_preserve_symbol_and_direction_and_numbers_attempts() {
    let broker = MockBroker::new();
    broker.push_fault(Fault::Reject(10019));
    broker.push_fault(Fault::Reject(10019));
    let mut mgr = manager();
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "Q"));
    assert!(mgr
        .retry_order(&broker, "Q", OrderRequest::sell("EURUSD", 0.5))
        .is_err());
    assert!(mgr
        .retry_order(&broker, "Q", OrderRequest::buy("GBPUSD", 0.5))
        .is_err());
    assert!(mgr
        .retry_order(&broker, "Q", OrderRequest::buy("EURUSD", 0.5))
        .is_err()); // rejected again (fault #2)
    let third = mgr
        .retry_order(&broker, "Q", OrderRequest::buy("EURUSD", 0.5))
        .unwrap();
    assert_eq!(
        third.client_order_id, "Q-r2",
        "attempts are numbered, never reused"
    );
}

#[test]
fn reconcile_until_settled_waits_for_delayed_visibility_and_reports_timeouts() {
    use std::time::Duration;
    let broker = MockBroker::new();
    broker.push_fault(Fault::ExecuteThenLoseResponse);
    broker.set_visibility_delay(3);
    let mut mgr = OrderManager::with_policy(
        MAGIC,
        SafetyPolicy {
            min_absent_observations: 10,
            ..instant_policy()
        },
    );
    let _ = mgr.submit_order(&broker, buy("EURUSD", 0.5, "S"));
    let r = mgr
        .reconcile_until_settled(&broker, Duration::from_millis(1), Duration::from_secs(5))
        .unwrap();
    assert!(r.is_settled());
    assert_eq!(mgr.get_order("S").unwrap().state, OrderState::Reconciled);

    // A genuinely absent order under the default 30 s policy times out unresolved (and stays blocking).
    let broker2 = MockBroker::new();
    broker2.push_fault(Fault::LoseBeforeExecution);
    let mut mgr2 = OrderManager::new(MAGIC);
    let _ = mgr2.submit_order(&broker2, buy("EURUSD", 0.5, "T"));
    let r2 = mgr2
        .reconcile_until_settled(
            &broker2,
            Duration::from_millis(5),
            Duration::from_millis(40),
        )
        .unwrap();
    assert!(!r2.is_settled());
    assert_eq!(mgr2.lifecycle(), LifecycleState::Degraded);
}

#[test]
fn ea_or_mt5_restart_loses_the_ea_cache_but_manager_idempotency_still_holds() {
    let broker = MockBroker::new();
    let mut mgr = manager();
    mgr.submit_order(&broker, buy("EURUSD", 0.5, "E")).unwrap();
    broker.restart_ea(); // EA idempotency cache gone
    let again = mgr.submit_order(&broker, buy("EURUSD", 0.5, "E")).unwrap();
    assert_eq!(again.state, OrderState::Filled);
    assert_eq!(
        broker.executions(),
        1,
        "manager-level idempotency does not depend on the EA cache"
    );
    assert_eq!(broker.send_calls(), 1);
}

#[test]
fn ea_cache_protects_a_raw_replay_but_only_until_the_ea_restarts() {
    // Why the EA cache is defence-in-depth, not the guarantee: after a restart a raw resend duplicates.
    let broker = MockBroker::new();
    let req = buy("EURUSD", 0.5, "raw-1");
    broker.order_send(&req).unwrap();
    broker.order_send(&req).unwrap();
    assert_eq!(
        broker.executions(),
        1,
        "EA cache replays the recorded result"
    );
    broker.restart_ea();
    broker.order_send(&req).unwrap();
    assert_eq!(broker.executions(), 2, "…but not after the EA restarts");
}

// ═════════════════════════════════════════════════════════════════════════════
// Concurrency contract
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn manager_types_are_send_and_shared_handle_is_sync() {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}
    send::<OrderManager>();
    send::<SharedOrderManager>();
    sync::<SharedOrderManager>();
}

#[test]
fn shared_manager_same_client_order_id_from_many_threads_reaches_the_broker_once() {
    const N: usize = 32;
    let broker = Arc::new(MockBroker::new());
    broker.set_ea_idempotency(false); // remove the EA safety net: the manager alone must hold
    let shared = SharedOrderManager::new(manager());
    let barrier = Arc::new(Barrier::new(N));

    let handles: Vec<_> = (0..N)
        .map(|_| {
            let (b, s, bar) = (broker.clone(), shared.clone(), barrier.clone());
            std::thread::spawn(move || {
                bar.wait();
                s.submit_order(&b, buy("EURUSD", 0.1, "shared-dup"))
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<TrackedOrder> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    assert_eq!(
        broker.executions(),
        1,
        "exactly one broker execution for one client_order_id"
    );
    assert_eq!(broker.send_calls(), 1);
    let first = results[0].order_ticket;
    assert!(results
        .iter()
        .all(|r| r.order_ticket == first && r.state == OrderState::Filled));
}

#[test]
fn shared_manager_distinct_ids_from_many_threads_all_execute() {
    const N: usize = 32;
    let broker = Arc::new(MockBroker::new());
    let shared = SharedOrderManager::new(manager());
    let handles: Vec<_> = (0..N)
        .map(|i| {
            let (b, s) = (broker.clone(), shared.clone());
            std::thread::spawn(move || {
                s.submit_order(&b, buy("EURUSD", 0.1, &format!("par-{i}")))
                    .unwrap()
            })
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap().state, OrderState::Filled);
    }
    assert_eq!(broker.executions(), N);
    assert_eq!(shared.export_orders().len(), N);
}

#[test]
fn documented_misuse_releasing_the_lock_between_check_and_send_duplicates_orders() {
    // The failure mode the README warns about: a hand-rolled Mutex<OrderManager> that is dropped
    // after the idempotency check and before the network call. A barrier makes the race
    // deterministic: every thread passes the check before any thread sends.
    const N: usize = 8;
    let broker = Arc::new(MockBroker::new());
    broker.set_ea_idempotency(false);
    let mgr = Arc::new(Mutex::new(manager()));
    let barrier = Arc::new(Barrier::new(N));

    let handles: Vec<_> = (0..N)
        .map(|_| {
            let (b, m, bar) = (broker.clone(), mgr.clone(), barrier.clone());
            std::thread::spawn(move || {
                let mut req = buy("EURUSD", 0.1, "misuse-1");
                let seen_before = m
                    .lock()
                    .unwrap()
                    .check_idempotency_and_magic(&mut req)
                    .unwrap();
                // lock released here ↓ — the bug
                bar.wait();
                if seen_before.is_none() {
                    b.order_send(&req).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(
        broker.executions(),
        N,
        "every thread saw 'new' and sent: N executions for ONE id"
    );
}

#[test]
fn a_panic_mid_submission_leaves_a_recoverable_state_and_does_not_poison_the_shared_manager() {
    struct PanicOnSend<'a>(&'a MockBroker);
    impl TradingBackend for PanicOnSend<'_> {
        fn order_send(&self, _r: &OrderRequest) -> Result<TradeResult> {
            panic!("simulated crash inside order_send");
        }
        fn order_close_with_magic(&self, t: u64, m: u64) -> Result<TradeResult> {
            self.0.order_close_with_magic(t, m)
        }
        fn order_modify_with_magic(&self, t: u64, m: u64, sl: f64, tp: f64) -> Result<TradeResult> {
            self.0.order_modify_with_magic(t, m, sl, tp)
        }
        fn positions_filtered(&self, m: Option<u64>, s: Option<&str>) -> Result<Vec<Position>> {
            self.0.positions_filtered(m, s)
        }
        fn pending_orders_filtered(
            &self,
            m: Option<u64>,
            s: Option<&str>,
        ) -> Result<Vec<WorkingOrder>> {
            self.0.pending_orders_filtered(m, s)
        }
        fn deals_filtered(
            &self,
            f: i64,
            t: i64,
            m: Option<u64>,
            s: Option<&str>,
        ) -> Result<Vec<Deal>> {
            self.0.deals_filtered(f, t, m, s)
        }
    }

    let broker = Arc::new(MockBroker::new());
    let shared = SharedOrderManager::new(manager());
    let (b, s) = (broker.clone(), shared.clone());
    let crashed = std::thread::spawn(move || {
        let _ = s.submit_order(&PanicOnSend(&b), buy("EURUSD", 0.5, "crash-1"));
    })
    .join();
    assert!(
        crashed.is_err(),
        "the worker really panicked while holding the lock"
    );

    // The lock is recovered, and the interrupted order is visibly Submitting → resolvable.
    assert_eq!(
        shared.get_order("crash-1").unwrap().state,
        OrderState::Submitting
    );
    let report = shared.reconcile(&*broker).unwrap();
    assert_eq!(
        report.absent_orders.len(),
        1,
        "nothing reached the broker → confirmed absent"
    );
    assert_eq!(
        shared.get_order("crash-1").unwrap().state,
        OrderState::Rejected
    );
}

#[test]
fn arc_and_reference_backends_are_accepted_by_the_manager() {
    let arc = Arc::new(MockBroker::new());
    let mut mgr = manager();
    mgr.submit_order(&arc, buy("EURUSD", 0.1, "arc-1")).unwrap(); // &Arc<MockBroker>
    let by_ref: &MockBroker = &arc;
    mgr.submit_order(&by_ref, buy("EURUSD", 0.1, "ref-1"))
        .unwrap(); // &&MockBroker
    assert_eq!(arc.executions(), 2);
}

// ═════════════════════════════════════════════════════════════════════════════
// Durable persistence and crash recovery
// ═════════════════════════════════════════════════════════════════════════════

fn store_at(path: &std::path::Path) -> Box<dyn OrderStore> {
    Box::new(JsonFileStore::new(path))
}

#[test]
fn restart_after_a_lost_response_resumes_from_disk_and_reconciles() {
    let dir = scratch_dir("restart");
    let path = dir.join("orders.json");
    let broker = MockBroker::new();
    broker.push_fault(Fault::ExecuteThenLoseResponse);

    {
        let mut mgr = OrderManager::with_store(MAGIC, store_at(&path)).unwrap();
        mgr.set_policy(instant_policy());
        assert!(is_unknown(
            &mgr.submit_order(&broker, buy("EURUSD", 0.5, "durable-1"))
                .unwrap_err()
        ));
    } // process "exits"

    let mut mgr = OrderManager::with_store(MAGIC, store_at(&path)).unwrap();
    mgr.set_policy(instant_policy());
    assert_eq!(
        mgr.get_order("durable-1").unwrap().state,
        OrderState::Unknown
    );
    // …and the restored Unknown order blocks a naive re-submission under a NEW id:
    assert!(matches!(
        mgr.submit_order(&broker, buy("EURUSD", 0.5, "durable-2"))
            .unwrap_err(),
        Mt5Error::UnresolvedUnknownOrders { .. }
    ));

    mgr.reconcile(&broker).unwrap();
    assert_eq!(
        mgr.get_order("durable-1").unwrap().state,
        OrderState::Reconciled
    );
    assert_eq!(broker.executions(), 1);

    // the resolution itself was journaled
    let mgr2 = OrderManager::with_store(MAGIC, store_at(&path)).unwrap();
    assert_eq!(
        mgr2.get_order("durable-1").unwrap().state,
        OrderState::Reconciled
    );
    assert_eq!(
        mgr2.get_order("durable-1").unwrap().wire_id,
        wire_id("durable-1")
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_crash_between_write_ahead_and_outcome_restores_as_unknown() {
    struct CrashOnSend<'a>(&'a MockBroker);
    impl TradingBackend for CrashOnSend<'_> {
        fn order_send(&self, r: &OrderRequest) -> Result<TradeResult> {
            // The request reaches the broker, then the process dies before the outcome is recorded.
            let _ = self.0.order_send(r);
            panic!("simulated process death after transmission");
        }
        fn order_close_with_magic(&self, t: u64, m: u64) -> Result<TradeResult> {
            self.0.order_close_with_magic(t, m)
        }
        fn order_modify_with_magic(&self, t: u64, m: u64, sl: f64, tp: f64) -> Result<TradeResult> {
            self.0.order_modify_with_magic(t, m, sl, tp)
        }
        fn positions_filtered(&self, m: Option<u64>, s: Option<&str>) -> Result<Vec<Position>> {
            self.0.positions_filtered(m, s)
        }
        fn pending_orders_filtered(
            &self,
            m: Option<u64>,
            s: Option<&str>,
        ) -> Result<Vec<WorkingOrder>> {
            self.0.pending_orders_filtered(m, s)
        }
        fn deals_filtered(
            &self,
            f: i64,
            t: i64,
            m: Option<u64>,
            s: Option<&str>,
        ) -> Result<Vec<Deal>> {
            self.0.deals_filtered(f, t, m, s)
        }
    }
    let dir = scratch_dir("crash");
    let path = dir.join("orders.json");
    let broker = MockBroker::new();

    let mut mgr = OrderManager::with_store(MAGIC, store_at(&path)).unwrap();
    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = mgr.submit_order(&CrashOnSend(&broker), buy("EURUSD", 0.5, "wal-1"));
    }));
    assert!(died.is_err());
    assert_eq!(
        broker.executions(),
        1,
        "the order reached the broker before the 'crash'"
    );
    drop(mgr); // the in-memory state is gone; only the file remains

    // The file holds the write-ahead Submitting record; a fresh process restores it as Unknown.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.contains("Submitting"),
        "write-ahead record must be on disk before transmission"
    );
    let mut mgr = OrderManager::with_store(MAGIC, store_at(&path)).unwrap();
    mgr.set_policy(instant_policy());
    assert_eq!(mgr.get_order("wal-1").unwrap().state, OrderState::Unknown);
    mgr.reconcile(&broker).unwrap();
    assert_eq!(
        mgr.get_order("wal-1").unwrap().state,
        OrderState::Reconciled,
        "found via position/deal"
    );
    assert_eq!(broker.executions(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn if_the_write_ahead_record_cannot_be_saved_the_order_is_not_transmitted() {
    let dir = scratch_dir("nowal");
    let path = dir.join("missing-subdir").join("orders.json"); // parent does not exist → save fails
    let broker = MockBroker::new();
    let mut mgr = OrderManager::with_store(MAGIC, store_at(&path)).unwrap(); // load of absent file = empty
    let err = mgr
        .submit_order(&broker, buy("EURUSD", 0.5, "nowal-1"))
        .unwrap_err();
    assert!(matches!(err, Mt5Error::PersistenceError(_)), "{err:?}");
    assert_eq!(
        broker.send_calls(),
        0,
        "no durable record ⇒ nothing may be sent"
    );
    assert!(
        mgr.get_order("nowal-1").is_none(),
        "in-memory record rolled back so the caller can retry"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_or_foreign_journal_files_are_an_error_never_an_empty_order_book() {
    let dir = scratch_dir("corrupt");
    let path = dir.join("orders.json");
    for bad in [
        "{ this is not json".to_string(),
        r#"{"hello":"world"}"#.to_string(),
        r#"{"format":"something/else","version":1,"saved_at":0,"orders":[]}"#.to_string(),
        r#"{"format":"mt5-bridge/orders","version":99,"saved_at":0,"orders":[]}"#.to_string(),
        String::new(),
    ] {
        std::fs::write(&path, &bad).unwrap();
        let r = OrderManager::with_store(MAGIC, store_at(&path));
        assert!(
            matches!(r, Err(Mt5Error::PersistenceError(_))),
            "accepted bad journal: {bad:?}"
        );
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_stale_temp_file_from_a_crash_mid_write_is_ignored_and_overwritten() {
    let dir = scratch_dir("tmpfile");
    let path = dir.join("orders.json");
    let store = JsonFileStore::new(&path);

    let broker = MockBroker::new();
    let mut mgr = OrderManager::with_store(MAGIC, Box::new(store.clone())).unwrap();
    mgr.submit_order(&broker, buy("EURUSD", 0.1, "tmp-1"))
        .unwrap();

    // Simulate a crash halfway through a later save: garbage left in the temp file.
    std::fs::write(dir.join("orders.json.tmp"), b"{ truncated garb").unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(
        loaded.len(),
        1,
        "the committed file is untouched by a torn temp file"
    );

    mgr.submit_order(&broker, buy("EURUSD", 0.1, "tmp-2"))
        .unwrap(); // next save overwrites the temp file
    assert_eq!(store.load().unwrap().len(), 2);
    assert!(
        !dir.join("orders.json.tmp").exists(),
        "temp file is consumed by the atomic rename"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn journal_file_is_a_versioned_envelope_and_a_fresh_store_loads_empty() {
    let dir = scratch_dir("envelope");
    let path = dir.join("orders.json");
    let store = JsonFileStore::new(&path);
    assert!(
        store.load().unwrap().is_empty(),
        "never-written store = first run"
    );
    let broker = MockBroker::new();
    let mut mgr = OrderManager::with_store(MAGIC, Box::new(store)).unwrap();
    mgr.submit_order(&broker, buy("EURUSD", 0.1, "env-1"))
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["format"], "mt5-bridge/orders");
    assert_eq!(v["version"], 1);
    assert_eq!(v["orders"][0]["client_order_id"], "env-1");
    std::fs::remove_dir_all(dir).ok();
}

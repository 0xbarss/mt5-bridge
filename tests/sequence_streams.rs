#![cfg(feature = "async")]
use mt5_bridge::stream::{BookSubscription, EventBus, SequenceTracker};
use mt5_bridge::{BookEvent, StreamError, StreamMode, Tick, TickSubscription};
use std::sync::Arc;

#[test]
fn test_sequence_tracker_monotonic_and_gaps() {
    let mut tracker = SequenceTracker::default();
    assert_eq!(tracker.last_seq(), None);
    assert_eq!(tracker.gaps(), 0);
    assert_eq!(tracker.reordered(), 0);

    // Sequence 0 is treated as unsequenced / ignored
    assert_eq!(tracker.observe(0), None);
    assert_eq!(tracker.last_seq(), None);

    // Initial sequence
    assert_eq!(tracker.observe(1), None);
    assert_eq!(tracker.last_seq(), Some(1));

    // Contiguous sequence
    assert_eq!(tracker.observe(2), None);
    assert_eq!(tracker.last_seq(), Some(2));

    // Forward gap: missing 3 and 4, got 5
    assert_eq!(tracker.observe(5), Some((3, 4)));
    assert_eq!(tracker.last_seq(), Some(5));
    assert_eq!(tracker.gaps(), 1);

    // Reordered arrival: seq 3 arrives after 5 (or 3 after 2)
    assert_eq!(tracker.observe(3), None);
    assert_eq!(tracker.reordered(), 1);
    assert_eq!(tracker.gaps(), 1);
    assert_eq!(tracker.last_seq(), Some(5));

    // Contiguous sequence continues from high-water mark (6 after 5)
    assert_eq!(tracker.observe(6), None);
    assert_eq!(tracker.last_seq(), Some(6));
}

#[tokio::test]
async fn test_book_subscription_dispatch_gap_and_drain() {
    let bus = EventBus::new(64);
    let rx = bus.subscribe_book("EURUSD");
    let mut sub = BookSubscription::new("EURUSD", rx);

    // Dispatch seq 1, 2, 5
    for (seq, price) in [(1u64, 1.0850), (2, 1.0851), (5, 1.0854)] {
        bus.dispatch_book(BookEvent {
            symbol: "EURUSD".to_string(),
            time_msc: 1710000000000,
            is_buy: true,
            price,
            volume: 10.0,
            event_seq: seq,
        });
    }

    // Yields seq 1
    let ev1 = sub.recv_checked().await.expect("recv 1").expect("some ev");
    assert_eq!(ev1.event_seq, 1);
    assert_eq!(sub.gaps(), 0);

    // Yields seq 2
    let ev2 = sub.recv_checked().await.expect("recv 2").expect("some ev");
    assert_eq!(ev2.event_seq, 2);
    assert_eq!(sub.gaps(), 0);

    // Seq 5 triggers Gap { expected: 3, got: 5 }
    let err = sub.recv_checked().await.expect_err("gap error expected");
    assert_eq!(
        err,
        StreamError::Gap {
            expected: 3,
            got: 5
        }
    );
    assert_eq!(sub.gaps(), 1);

    // Subsequent recv_checked yields seq 5 without silent loss
    let ev5 = sub
        .recv_checked()
        .await
        .expect("recv 5 after gap")
        .expect("some ev");
    assert_eq!(ev5.event_seq, 5);
    assert_eq!(sub.last_seq(), Some(5));
}

#[tokio::test]
async fn test_book_subscription_prunes_idle_sender_on_drop() {
    let bus = Arc::new(EventBus::new(64));
    let rx = bus.subscribe_book("GBPUSD");
    let sub = BookSubscription::with_bus("GBPUSD", rx, Arc::clone(&bus));

    assert!(bus.active_book_symbols().contains(&"GBPUSD".to_string()));

    // Dropping subscription prunes the entry from EventBus
    drop(sub);
    assert!(!bus.active_book_symbols().contains(&"GBPUSD".to_string()));
}

#[tokio::test]
async fn test_tick_subscription_observes_sequence_and_gaps() {
    let bus = EventBus::new(64);
    let rx = bus.subscribe_ticks("USDJPY");
    let mut sub = TickSubscription::new("USDJPY", StreamMode::Lossless, rx);

    assert_eq!(sub.last_seq(), None);
    assert_eq!(sub.gaps(), 0);

    for (seq, bid) in [(1u64, 150.00), (2, 150.01), (5, 150.04)] {
        bus.dispatch_tick(Tick {
            symbol: "USDJPY".to_string(),
            time: 1710000000,
            time_msc: 1710000000000,
            bid,
            ask: bid + 0.01,
            last: bid,
            volume: 1,
            flags: 6,
            event_seq: seq,
        });
    }

    let t1 = sub.recv().await.expect("tick 1");
    assert_eq!(t1.event_seq, 1);
    assert_eq!(sub.last_seq(), Some(1));

    let t2 = sub.recv().await.expect("tick 2");
    assert_eq!(t2.event_seq, 2);
    assert_eq!(sub.last_seq(), Some(2));

    let t5 = sub.recv().await.expect("tick 5");
    assert_eq!(t5.event_seq, 5);
    assert_eq!(sub.last_seq(), Some(5));
    assert_eq!(sub.gaps(), 1);
}

/// 1,000 subscribe/unsubscribe cycles must leave the tick sender map empty:
/// no unbounded growth in the sender maps.
#[tokio::test]
async fn test_thousand_subscribe_unsubscribe_cycles_leave_no_active_tick_symbols() {
    let bus = EventBus::new(64);
    for i in 0..1000 {
        let symbol = format!("SYM{}", i % 10);
        let rx = bus.subscribe_ticks(&symbol);
        drop(rx);
        bus.remove_tick_sender(&symbol);
    }
    assert!(
        bus.active_tick_symbols().is_empty(),
        "tick sender map must not grow across subscribe/unsubscribe cycles"
    );
}

/// An event pushed immediately after subscribe must be received: subscribing
/// the bus before the EA acknowledges removes the startup race that could
/// otherwise drop early ticks.
#[tokio::test]
async fn test_event_sent_immediately_after_subscribe_is_received() {
    let bus = EventBus::new(64);
    let rx = bus.subscribe_ticks("EURUSD");
    let mut sub = TickSubscription::new("EURUSD", StreamMode::Latest, rx);

    bus.dispatch_tick(Tick {
        symbol: "EURUSD".into(),
        time: 1710000000,
        time_msc: 1710000000000,
        bid: 1.0850,
        ask: 1.0851,
        last: 1.0850,
        volume: 1,
        flags: 6,
        event_seq: 1,
    });

    let tick = sub.recv().await.expect("tick expected");
    assert_eq!(tick.event_seq, 1);
}

/// `prune()` sweeps idle senders from all maps so symbols with no remaining
/// receivers do not linger indefinitely.
#[test]
fn test_prune_removes_idle_tick_senders() {
    let bus = EventBus::new(64);

    let _rx_a1 = bus.subscribe_ticks("A");
    let _rx_a2 = bus.subscribe_ticks("A");
    let _rx_b = bus.subscribe_ticks("B");
    drop(_rx_a2);
    drop(_rx_b);

    bus.prune();

    assert!(bus.active_tick_symbols().contains(&"A".to_string()));
    assert!(!bus.active_tick_symbols().contains(&"B".to_string()));
}

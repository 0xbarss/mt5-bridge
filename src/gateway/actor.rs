//! Dedicated MT5 Gateway actor with priority queues and event streaming.

use super::executor::BlockingExecutor;
use super::{Connector, FixedConnector, GatewayConfig};
use crate::backend::TradingBackend;
use crate::error::{Mt5Error, Result};
use crate::position_book::PositionBook;
use crate::reconciliation::{
    classify_send_error, LifecycleState, OrderEvent, OrderManager, OrderObserver,
    ReconciliationReport, SharedOrderManager,
};
use crate::risk::{PreTradeDecision, RiskEngine};
use crate::session::{Mt5Session, SessionConfig};
use crate::types::{
    AccountInfo, Deal, HistoryResult, Level, OrderRequest, OrderState, Position, Timeframe,
    TrackedOrder, TradeEvent, TradeResult, WorkingOrder,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::{debug, info, warn};

/// Priority 1 (Latency-Critical): order submissions, closes, and SL/TP modifications.
#[derive(Debug)]
pub enum P1Command {
    SubmitOrder {
        req: OrderRequest,
        tx: oneshot::Sender<Result<TrackedOrder>>,
    },
    ClosePosition {
        ticket: u64,
        magic: u64,
        tx: oneshot::Sender<Result<TradeResult>>,
    },
    ModifyOrder {
        ticket: u64,
        magic: u64,
        stop_loss: Level,
        take_profit: Level,
        tx: oneshot::Sender<Result<TradeResult>>,
    },
}

/// Priority 2 (Transactional): state queries, explicit reconciliation, and health checks.
#[derive(Debug)]
pub enum P2Command {
    AccountInfo {
        tx: oneshot::Sender<Result<AccountInfo>>,
    },
    GetPositions {
        magic: Option<u64>,
        symbol: Option<String>,
        tx: oneshot::Sender<Result<Vec<Position>>>,
    },
    GetPendingOrders {
        magic: Option<u64>,
        symbol: Option<String>,
        tx: oneshot::Sender<Result<Vec<WorkingOrder>>>,
    },
    GetDeals {
        from: i64,
        to: i64,
        magic: Option<u64>,
        symbol: Option<String>,
        tx: oneshot::Sender<Result<Vec<Deal>>>,
    },
    Reconcile {
        tx: oneshot::Sender<Result<ReconciliationReport>>,
    },
    GetPositionBook {
        tx: oneshot::Sender<Result<PositionBook>>,
    },
    Ping {
        tx: oneshot::Sender<Result<()>>,
    },
    RegisterTickSubscription {
        symbol: String,
    },
    UnregisterTickSubscription {
        symbol: String,
    },
    RegisterTradeSubscription,
    UnregisterTradeSubscription,
    RegisterBookSubscription {
        symbol: String,
    },
    UnregisterBookSubscription {
        symbol: String,
    },
    RegisterBarsSubscription {
        symbol: String,
        timeframe: Timeframe,
    },
    UnregisterBarsSubscription {
        symbol: String,
        timeframe: Timeframe,
    },
}

/// Priority 3 (Bulk): historical data requests.
#[derive(Debug)]
pub enum P3Command {
    CopyRates {
        symbol: String,
        timeframe: Timeframe,
        from: i64,
        to: i64,
        tx: oneshot::Sender<Result<HistoryResult>>,
        cancel: Arc<AtomicBool>,
    },
}

/// Cloneable handle to the running MT5 gateway actor.
#[derive(Clone)]
pub struct Mt5GatewayHandle {
    p1_tx: mpsc::Sender<P1Command>,
    p2_tx: mpsc::Sender<P2Command>,
    p3_tx: mpsc::Sender<P3Command>,
    trade_event_in_tx: mpsc::UnboundedSender<TradeEvent>,
    trade_event_out_tx: broadcast::Sender<TradeEvent>,
    order_event_out_tx: broadcast::Sender<OrderEvent>,
    tick_watchers: Arc<Mutex<HashMap<String, watch::Sender<Option<crate::types::Tick>>>>>,
    state_watch: watch::Receiver<LifecycleState>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl Mt5GatewayHandle {
    /// Submit a new order through the Priority 1 queue.
    ///
    /// Rejected immediately if the gateway session is not in `Ready` state.
    pub async fn submit_order(&self, req: OrderRequest) -> Result<TrackedOrder> {
        let (tx, rx) = oneshot::channel();
        self.p1_tx
            .send(P1Command::SubmitOrder { req, tx })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Close an existing position through the Priority 1 queue.
    pub async fn close_position(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        let (tx, rx) = oneshot::channel();
        self.p1_tx
            .send(P1Command::ClosePosition { ticket, magic, tx })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Modify SL/TP of an order or position through the Priority 1 queue.
    pub async fn modify_order(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: Level,
        take_profit: Level,
    ) -> Result<TradeResult> {
        let (tx, rx) = oneshot::channel();
        self.p1_tx
            .send(P1Command::ModifyOrder {
                ticket,
                magic,
                stop_loss,
                take_profit,
                tx,
            })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Query account information through the Priority 2 queue.
    pub async fn account_info(&self) -> Result<AccountInfo> {
        let (tx, rx) = oneshot::channel();
        self.p2_tx
            .send(P2Command::AccountInfo { tx })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Query open positions through the Priority 2 queue.
    pub async fn positions_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        let (tx, rx) = oneshot::channel();
        self.p2_tx
            .send(P2Command::GetPositions {
                magic,
                symbol: symbol.map(|s| s.to_string()),
                tx,
            })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Query pending orders through the Priority 2 queue.
    pub async fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        let (tx, rx) = oneshot::channel();
        self.p2_tx
            .send(P2Command::GetPendingOrders {
                magic,
                symbol: symbol.map(|s| s.to_string()),
                tx,
            })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Query deals through the Priority 2 queue.
    pub async fn deals_filtered(
        &self,
        from: i64,
        to: i64,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        let (tx, rx) = oneshot::channel();
        self.p2_tx
            .send(P2Command::GetDeals {
                from,
                to,
                magic,
                symbol: symbol.map(|s| s.to_string()),
                tx,
            })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Explicitly trigger order reconciliation through the Priority 2 queue.
    pub async fn reconcile(&self) -> Result<ReconciliationReport> {
        let (tx, rx) = oneshot::channel();
        self.p2_tx
            .send(P2Command::Reconcile { tx })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Query authoritative position book state through the Priority 2 queue.
    pub async fn position_book(&self) -> Result<PositionBook> {
        let (tx, rx) = oneshot::channel();
        self.p2_tx
            .send(P2Command::GetPositionBook { tx })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Check actor responsiveness and health through the Priority 2 queue.
    pub async fn ping(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.p2_tx
            .send(P2Command::Ping { tx })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Fetch historical rates through the Priority 3 queue.
    pub async fn copy_rates(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<HistoryResult> {
        let (tx, rx) = oneshot::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.p3_tx
            .send(P3Command::CopyRates {
                symbol: symbol.to_string(),
                timeframe,
                from,
                to,
                tx,
                cancel,
            })
            .await
            .map_err(|_| Mt5Error::ChannelDisconnected)?;
        rx.await.map_err(|_| Mt5Error::ChannelDisconnected)?
    }

    /// Ingest a trade transaction event into the bounded fan-out queue.
    pub fn dispatch_trade_event(&self, event: TradeEvent) {
        let _ = self.trade_event_in_tx.send(event);
    }

    /// Ingest a tick event and update conflated per-symbol watch channels in-place.
    pub fn dispatch_tick(&self, tick: crate::types::Tick) {
        let mut watchers = self
            .tick_watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(sender) = watchers.get(&tick.symbol) {
            let _ = sender.send(Some(tick));
        } else {
            let (tx, _rx) = watch::channel(Some(tick.clone()));
            watchers.insert(tick.symbol.clone(), tx);
        }
    }

    /// Subscribe to conflated tick updates for a symbol.
    pub fn subscribe_ticks(&self, symbol: &str) -> watch::Receiver<Option<crate::types::Tick>> {
        self.register_tick_subscription(symbol);
        let mut watchers = self
            .tick_watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(sender) = watchers.get(symbol) {
            sender.subscribe()
        } else {
            let (tx, rx) = watch::channel(None);
            watchers.insert(symbol.to_string(), tx);
            rx
        }
    }

    /// Subscribe to the bounded, ordered fan-out stream of trade events.
    ///
    /// Returns a [`TradeSubscription`](crate::stream::TradeSubscription) wrapping the broadcast
    /// receiver with explicit consumer-side lag detection (`recv_checked()` returns
    /// [`crate::stream::StreamError::Lagged`] when the broadcast buffer overflows). Trade event
    /// delivery is not lossless; a slow subscriber skips events and must handle lag explicitly.
    pub fn subscribe_trade_events(&self) -> crate::stream::TradeSubscription {
        crate::stream::TradeSubscription::new(self.trade_event_out_tx.subscribe())
    }

    /// Subscribe to order lifecycle events emitted by the gateway's order manager.
    ///
    /// Events are broadcast: every subscriber receives its own copy and a slow subscriber
    /// is detached rather than blocking the gateway. Combine with
    /// [`Mt5GatewayHandle::submit_order`] to observe submissions, fills, rejections, and
    /// reconciliation outcomes without polling.
    pub fn subscribe_order_events(&self) -> broadcast::Receiver<OrderEvent> {
        self.order_event_out_tx.subscribe()
    }

    /// Current operational lifecycle state.
    pub fn state(&self) -> LifecycleState {
        *self.state_watch.borrow()
    }

    /// Current operational lifecycle state (alias for `state`).
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.state()
    }

    /// Subscribe to changes in the lifecycle state stream.
    pub fn state_stream(&self) -> watch::Receiver<LifecycleState> {
        self.state_watch.clone()
    }

    /// Wait until the gateway transitions to `LifecycleState::Ready`, timing out after `timeout`.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        tokio::time::timeout(timeout, async {
            let mut rx = self.state_watch.clone();
            loop {
                if *rx.borrow() == LifecycleState::Ready {
                    return Ok(());
                }
                rx.changed()
                    .await
                    .map_err(|_| Mt5Error::ChannelDisconnected)?;
            }
        })
        .await
        .map_err(|_| Mt5Error::Timeout {
            operation: "wait_ready".to_string(),
        })?
    }

    /// Check if the session is ready for trading operations.
    pub fn is_ready(&self) -> bool {
        self.state() == LifecycleState::Ready
    }

    /// Register a symbol tick subscription to be preserved across reconnects.
    pub fn register_tick_subscription(&self, symbol: &str) {
        let _ = self.p2_tx.try_send(P2Command::RegisterTickSubscription {
            symbol: symbol.to_string(),
        });
    }

    /// Unregister a symbol tick subscription.
    pub fn unregister_tick_subscription(&self, symbol: &str) {
        let _ = self.p2_tx.try_send(P2Command::UnregisterTickSubscription {
            symbol: symbol.to_string(),
        });
    }

    /// Register trade event subscription to be preserved across reconnects.
    pub fn register_trade_subscription(&self) {
        let _ = self.p2_tx.try_send(P2Command::RegisterTradeSubscription);
    }

    /// Unregister trade event subscription.
    pub fn unregister_trade_subscription(&self) {
        let _ = self.p2_tx.try_send(P2Command::UnregisterTradeSubscription);
    }

    /// Register depth-of-market book subscription.
    pub fn register_book_subscription(&self, symbol: &str) {
        let _ = self.p2_tx.try_send(P2Command::RegisterBookSubscription {
            symbol: symbol.to_string(),
        });
    }

    /// Unregister depth-of-market book subscription.
    pub fn unregister_book_subscription(&self, symbol: &str) {
        let _ = self.p2_tx.try_send(P2Command::UnregisterBookSubscription {
            symbol: symbol.to_string(),
        });
    }

    /// Register closed-bar subscription.
    pub fn register_bars_subscription(&self, symbol: &str, timeframe: Timeframe) {
        let _ = self.p2_tx.try_send(P2Command::RegisterBarsSubscription {
            symbol: symbol.to_string(),
            timeframe,
        });
    }

    /// Unregister closed-bar subscription.
    pub fn unregister_bars_subscription(&self, symbol: &str, timeframe: Timeframe) {
        let _ = self.p2_tx.try_send(P2Command::UnregisterBarsSubscription {
            symbol: symbol.to_string(),
            timeframe,
        });
    }

    /// Request graceful shutdown of the gateway actor.
    pub fn shutdown(&self) {
        let mut lock = self
            .shutdown_tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(tx) = lock.take() {
            let _ = tx.send(());
        }
    }
}

/// Dedicated actor thread/task owning the MT5 backend and order manager.
pub struct Mt5Gateway<B: TradingBackend + Send + Sync + 'static> {
    backend: Option<Arc<B>>,
    order_manager: SharedOrderManager,
    position_book: Arc<Mutex<PositionBook>>,
    executor: BlockingExecutor,
    session: Mt5Session,
    p1_rx: mpsc::Receiver<P1Command>,
    p2_rx: mpsc::Receiver<P2Command>,
    p3_rx: mpsc::Receiver<P3Command>,
    trade_event_in_rx: mpsc::UnboundedReceiver<TradeEvent>,
    trade_event_out_tx: broadcast::Sender<TradeEvent>,
    _tick_watchers: Arc<Mutex<HashMap<String, watch::Sender<Option<crate::types::Tick>>>>>,
    state_watch: watch::Sender<LifecycleState>,
    last_event_seq: u64,
    shutdown_rx: oneshot::Receiver<()>,
    current_p3_cancel: Option<Arc<AtomicBool>>,
    disconnect_trigger_tx: mpsc::Sender<()>,
    disconnect_trigger_rx: mpsc::Receiver<()>,
    p1_timeout: Duration,
    p2_timeout: Duration,
    p3_timeout: Duration,
    order_event_out_tx: broadcast::Sender<OrderEvent>,
    risk_engine: Option<Arc<RiskEngine>>,
}

impl<B: TradingBackend + Send + Sync + 'static> Mt5Gateway<B> {
    /// One-shot backend; reconnect attempts return an error.
    pub fn spawn(
        backend: B,
        order_manager: OrderManager,
        mut session_config: SessionConfig,
    ) -> (Mt5GatewayHandle, tokio::task::JoinHandle<()>) {
        session_config.max_reconnect_attempts = Some(0);
        Self::spawn_supervised(FixedConnector::new(backend), order_manager, session_config)
    }

    /// Spawn the MT5 Gateway actor with full configuration options using a one-shot backend.
    pub fn spawn_with_config(
        backend: B,
        order_manager: OrderManager,
        mut config: GatewayConfig,
    ) -> (Mt5GatewayHandle, tokio::task::JoinHandle<()>) {
        config.session.max_reconnect_attempts = Some(0);
        Self::spawn_supervised_with_config(FixedConnector::new(backend), order_manager, config)
    }

    /// Supervised backend; reconnects with backoff.
    pub fn spawn_supervised<C: Connector<B>>(
        connector: C,
        order_manager: OrderManager,
        session_config: SessionConfig,
    ) -> (Mt5GatewayHandle, tokio::task::JoinHandle<()>) {
        Self::spawn_supervised_with_config(
            connector,
            order_manager,
            GatewayConfig {
                session: session_config,
                ..Default::default()
            },
        )
    }

    /// Supervised backend with full gateway configuration options.
    pub fn spawn_supervised_with_config<C: Connector<B>>(
        connector: C,
        order_manager: OrderManager,
        config: GatewayConfig,
    ) -> (Mt5GatewayHandle, tokio::task::JoinHandle<()>) {
        let (p1_tx, p1_rx) = mpsc::channel(1024);
        let (p2_tx, p2_rx) = mpsc::channel(1024);
        let (p3_tx, p3_rx) = mpsc::channel(1024);
        let (trade_event_in_tx, trade_event_in_rx) = mpsc::unbounded_channel();
        let (trade_event_out_tx, _) = broadcast::channel(2048);
        let (order_event_out_tx, _) = broadcast::channel(2048);
        let tick_watchers = Arc::new(Mutex::new(HashMap::new()));
        let (state_watch_tx, state_watch_rx) = watch::channel(LifecycleState::Starting);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (disconnect_trigger_tx, disconnect_trigger_rx) = mpsc::channel(32);

        let session = Mt5Session::new(config.session);
        let strategy_magic = order_manager.strategy_magic();
        let position_book = Arc::new(Mutex::new(PositionBook::new(Some(strategy_magic))));
        let executor = BlockingExecutor::new(config.executor_permits);

        // The handle is built first so it can act as the order manager's observer: every
        // lifecycle event is broadcast on `order_event_out_tx` for `subscribe_order_events()`.
        let handle = Mt5GatewayHandle {
            p1_tx,
            p2_tx,
            p3_tx,
            trade_event_in_tx,
            trade_event_out_tx: trade_event_out_tx.clone(),
            order_event_out_tx: order_event_out_tx.clone(),
            tick_watchers: Arc::clone(&tick_watchers),
            state_watch: state_watch_rx,
            shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
        };

        let order_manager = order_manager.with_observer(Box::new(handle.clone()));
        let shared_order_manager = SharedOrderManager::new(order_manager);

        let gateway = Self {
            backend: None,
            order_manager: shared_order_manager,
            position_book,
            executor,
            session,
            p1_rx,
            p2_rx,
            p3_rx,
            trade_event_in_rx,
            trade_event_out_tx: trade_event_out_tx.clone(),
            _tick_watchers: Arc::clone(&tick_watchers),
            state_watch: state_watch_tx,
            last_event_seq: 0,
            shutdown_rx,
            current_p3_cancel: None,
            disconnect_trigger_tx,
            disconnect_trigger_rx,
            p1_timeout: config.p1_timeout,
            p2_timeout: config.p2_timeout,
            p3_timeout: config.p3_timeout,
            order_event_out_tx,
            risk_engine: config.risk_engine,
        };

        let join_handle = tokio::spawn(async move {
            gateway.run_supervised(connector).await;
        });

        (handle, join_handle)
    }

    /// Supervisor loop managing connection, synchronization, reconciliation, and backoff retries.
    pub async fn run_supervised<C: Connector<B>>(mut self, connector: C) {
        let connector = Arc::new(connector);
        loop {
            self.session.set_state(LifecycleState::Connecting);
            self.publish_state().await;

            match connector.connect() {
                Ok(backend) => {
                    self.backend = Some(Arc::new(backend));
                    self.session.on_connected();
                    self.publish_state().await;

                    if let Err(e) = self.sync_symbols().await {
                        self.session.on_disconnect(&e.to_string());
                        self.backend = None;
                        self.publish_state().await;
                    } else {
                        self.session.on_synced();
                        self.publish_state().await;

                        if let Err(e) = self.reconcile_internal().await {
                            self.session.on_disconnect(&e.to_string());
                            self.backend = None;
                            self.publish_state().await;
                        } else {
                            self.session.on_reconciled();
                            self.publish_state().await;
                            self.replay_subscriptions().await;

                            self.serve_until_disconnect(&connector).await;
                            self.backend = None;
                            self.session.on_disconnect("pipe closed");
                            self.publish_state().await;
                        }
                    }
                }
                Err(e) => {
                    self.backend = None;
                    self.session.on_disconnect(&e.to_string());
                    self.publish_state().await;
                }
            }

            if matches!(self.session.state(), LifecycleState::Stopped) {
                break;
            }
            let delay = self.session.current_backoff();
            self.wait_backoff_or_command(delay).await;
            if matches!(self.session.state(), LifecycleState::Stopped) {
                break;
            }
        }
    }

    async fn publish_state(&mut self) {
        let state = self.session.state();
        self.order_manager.set_lifecycle(state);
        let _ = self.state_watch.send_replace(state);
        tokio::task::yield_now().await;
    }

    async fn sync_symbols(&self) -> Result<()> {
        Ok(())
    }

    async fn reconcile_internal(&self) -> Result<ReconciliationReport> {
        let Some(backend) = self.backend.clone() else {
            return Err(Mt5Error::PipeDisconnected);
        };
        let manager = self.order_manager.clone();
        let book = Arc::clone(&self.position_book);
        let executor = self.executor.clone();
        let timeout = self.p2_timeout;
        executor
            .run(timeout, "gateway_reconcile", move || {
                let res = manager.reconcile(&*backend);
                if let Ok(ref report) = res {
                    let _ = book
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .apply_snapshot(&report.positions);
                }
                res
            })
            .await
    }

    async fn replay_subscriptions(&self) {
        let Some(backend) = self.backend.clone() else {
            return;
        };

        let ticks: Vec<String> = self
            .session
            .active_tick_subscriptions()
            .iter()
            .cloned()
            .collect();
        for sym in ticks {
            let b = Arc::clone(&backend);
            let ex = self.executor.clone();
            let _ = ex
                .run(self.p2_timeout, "replay_ticks", move || {
                    b.subscribe_ticks(&sym)
                })
                .await;
        }

        if self.session.is_trade_subscribed() {
            let b = Arc::clone(&backend);
            let ex = self.executor.clone();
            let _ = ex
                .run(self.p2_timeout, "replay_trade", move || b.subscribe_trade())
                .await;
        }

        let books: Vec<String> = self
            .session
            .active_book_subscriptions()
            .iter()
            .cloned()
            .collect();
        for sym in books {
            let b = Arc::clone(&backend);
            let ex = self.executor.clone();
            let _ = ex
                .run(self.p2_timeout, "replay_book", move || {
                    b.subscribe_book(&sym)
                })
                .await;
        }

        let bars: Vec<(String, Timeframe)> = self
            .session
            .active_bar_subscriptions()
            .iter()
            .cloned()
            .collect();
        for (sym, tf) in bars {
            let b = Arc::clone(&backend);
            let ex = self.executor.clone();
            let _ = ex
                .run(self.p2_timeout, "replay_bars", move || {
                    b.subscribe_bars(&sym, tf)
                })
                .await;
        }
    }

    async fn wait_backoff_or_command(&mut self, delay: Duration) {
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                biased;
                _ = &mut self.shutdown_rx => {
                    info!("Mt5Gateway received shutdown signal during backoff");
                    self.session.set_state(LifecycleState::Stopped);
                    self.publish_state().await;
                    break;
                }
                _ = &mut sleep => {
                    break;
                }
                Some(cmd) = self.p1_rx.recv() => {
                    self.handle_p1(cmd);
                }
                Some(cmd) = self.p2_rx.recv() => {
                    self.handle_p2(cmd);
                }
                Some(cmd) = self.p3_rx.recv() => {
                    self.handle_p3(cmd);
                }
                Some(event) = self.trade_event_in_rx.recv() => {
                    self.handle_trade_event(event);
                }
            }
        }
    }

    async fn serve_until_disconnect<C: Connector<B>>(&mut self, connector: &Arc<C>) {
        let mut health_interval = tokio::time::interval(Duration::from_secs(5));
        health_interval.tick().await;

        let mut disconnect_rx = self.backend.as_ref().and_then(|b| b.subscribe_disconnect());

        loop {
            tokio::select! {
                biased;
                _ = &mut self.shutdown_rx => {
                    info!("Mt5Gateway received shutdown signal; stopping actor");
                    self.session.set_state(LifecycleState::Stopped);
                    self.publish_state().await;
                    break;
                }
                _ = self.disconnect_trigger_rx.recv() => {
                    warn!("Internal disconnect trigger fired; stopping serve loop");
                    break;
                }
                _ = async {
                    if let Some(ref mut rx) = disconnect_rx {
                        let _ = rx.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    warn!("Disconnect notification received from backend; stopping serve loop");
                    break;
                }
                _ = health_interval.tick() => {
                    if let Some(ref backend) = self.backend {
                        let b = Arc::clone(backend);
                        let c = Arc::clone(connector);
                        let executor = self.executor.clone();
                        let timeout = self.p2_timeout;
                        let health_res = executor.run(timeout, "health_check", move || {
                            c.health_check(&*b)
                        }).await;
                        if let Err(e) = health_res {
                            warn!(error = %e, "Supervisor periodic health check failed; initiating reconnect");
                            break;
                        }
                    }
                }
                Some(cmd) = self.p1_rx.recv() => {
                    self.handle_p1(cmd);
                }
                Some(cmd) = self.p2_rx.recv() => {
                    self.handle_p2(cmd);
                }
                Some(cmd) = self.p3_rx.recv() => {
                    self.handle_p3(cmd);
                }
                Some(event) = self.trade_event_in_rx.recv() => {
                    self.handle_trade_event(event);
                }
                else => {
                    debug!("All Mt5Gateway channels closed; stopping actor");
                    self.session.set_state(LifecycleState::Stopped);
                    self.publish_state().await;
                    break;
                }
            }
        }
    }

    fn cancel_active_p3(&self) {
        if let Some(cancel) = &self.current_p3_cancel {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    fn handle_p1(&mut self, cmd: P1Command) {
        self.cancel_active_p3();

        match cmd {
            P1Command::SubmitOrder { req, tx } => {
                if !self.session.is_ready() {
                    let err = Mt5Error::PreflightRejected(format!(
                        "MT5 session {:?} is not ready for new order submissions",
                        self.session.state()
                    ));
                    let _ = tx.send(Err(err));
                    return;
                }

                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };

                let manager = self.order_manager.clone();
                let executor = self.executor.clone();
                let timeout = self.p1_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                let risk_engine = self.risk_engine.clone();
                let strategy_magic = manager.strategy_magic();
                tokio::spawn(async move {
                    let res: Result<TrackedOrder, Mt5Error> = async {
                        // Pre-trade risk check: reject before any backend submission so a
                        // risk rejection never reaches the MT5 backend.
                        if let Some(engine) = &risk_engine {
                            let account = backend.account_info()?;
                            let positions = backend
                                .positions_filtered(Some(strategy_magic), Some(&req.symbol))?;
                            let working_orders = backend
                                .pending_orders_filtered(Some(strategy_magic), Some(&req.symbol))?;
                            match engine.pre_trade(&req, &account, &positions, &working_orders)? {
                                PreTradeDecision::Allowed => {}
                                PreTradeDecision::Rejected { rule, detail } => {
                                    return Err(Mt5Error::RiskRejected { rule, detail });
                                }
                            }
                        }

                        executor
                            .run(timeout, "p1_submit_order", move || {
                                manager.submit_order(&*backend, req)
                            })
                            .await
                    }
                    .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
            P1Command::ClosePosition { ticket, magic, tx } => {
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };

                let manager = self.order_manager.clone();
                let book = Arc::clone(&self.position_book);
                let executor = self.executor.clone();
                let timeout = self.p1_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p1_close_position", move || {
                            if magic > 0 && magic != manager.strategy_magic() {
                                backend.order_close_with_magic(ticket, magic)
                            } else {
                                book.lock()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .mark_closing(ticket);
                                let outcome = manager.close_position(&*backend, ticket);
                                if let Err(ref e) = outcome {
                                    let (new_state, _, desc) = classify_send_error(e);
                                    if new_state == OrderState::Unknown {
                                        book.lock()
                                            .unwrap_or_else(|p| p.into_inner())
                                            .mark_unknown(ticket, &desc);
                                    }
                                }
                                outcome
                            }
                        })
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
            P1Command::ModifyOrder {
                ticket,
                magic,
                stop_loss,
                take_profit,
                tx,
            } => {
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };

                let manager = self.order_manager.clone();
                let book = Arc::clone(&self.position_book);
                let executor = self.executor.clone();
                let timeout = self.p1_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p1_modify_order", move || {
                            if magic > 0 && magic != manager.strategy_magic() {
                                backend.order_modify_with_magic(
                                    ticket,
                                    magic,
                                    stop_loss,
                                    take_profit,
                                )
                            } else {
                                let outcome =
                                    manager.modify_order(&*backend, ticket, stop_loss, take_profit);
                                match &outcome {
                                    Ok(_) => {
                                        let mut b = book.lock().unwrap_or_else(|p| p.into_inner());
                                        if let Some(pos) = b.get_mut(ticket) {
                                            match stop_loss {
                                                Level::Set(v) => pos.stop_loss = v,
                                                Level::Clear => pos.stop_loss = 0.0,
                                                Level::Keep => {}
                                            }
                                            match take_profit {
                                                Level::Set(v) => pos.take_profit = v,
                                                Level::Clear => pos.take_profit = 0.0,
                                                Level::Keep => {}
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let (new_state, _, desc) = classify_send_error(e);
                                        if new_state == OrderState::Unknown {
                                            book.lock()
                                                .unwrap_or_else(|p| p.into_inner())
                                                .mark_unknown(ticket, &desc);
                                        }
                                    }
                                }
                                outcome
                            }
                        })
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
        }
    }

    fn handle_p2(&mut self, cmd: P2Command) {
        match cmd {
            P2Command::AccountInfo { tx } => {
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_account_info", move || backend.account_info())
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
            P2Command::GetPositions { magic, symbol, tx } => {
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_get_positions", move || {
                            backend.positions_filtered(magic, symbol.as_deref())
                        })
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
            P2Command::GetPendingOrders { magic, symbol, tx } => {
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_get_pending_orders", move || {
                            backend.pending_orders_filtered(magic, symbol.as_deref())
                        })
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
            P2Command::GetDeals {
                from,
                to,
                magic,
                symbol,
                tx,
            } => {
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_get_deals", move || {
                            backend.deals_filtered(from, to, magic, symbol.as_deref())
                        })
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
            P2Command::Reconcile { tx } => {
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };
                let manager = self.order_manager.clone();
                let book = Arc::clone(&self.position_book);
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_reconcile", move || {
                            let res = manager.reconcile(&*backend);
                            if let Ok(ref report) = res {
                                let _ = book
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .apply_snapshot(&report.positions);
                            }
                            res
                        })
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
            P2Command::GetPositionBook { tx } => {
                let book = self
                    .position_book
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                let _ = tx.send(Ok(book));
            }
            P2Command::Ping { tx } => {
                let _ = tx.send(Ok(()));
            }
            P2Command::RegisterTickSubscription { symbol } => {
                self.session.register_tick_subscription(&symbol);
                if let Some(ref backend) = self.backend {
                    let b = Arc::clone(backend);
                    let sym = symbol.clone();
                    let executor = self.executor.clone();
                    let timeout = self.p2_timeout;
                    tokio::spawn(async move {
                        let _ = executor
                            .run(timeout, "sub_ticks", move || b.subscribe_ticks(&sym))
                            .await;
                    });
                }
            }
            P2Command::UnregisterTickSubscription { symbol } => {
                self.session.unregister_tick_subscription(&symbol);
            }
            P2Command::RegisterTradeSubscription => {
                self.session.register_trade_subscription();
                if let Some(ref backend) = self.backend {
                    let b = Arc::clone(backend);
                    let executor = self.executor.clone();
                    let timeout = self.p2_timeout;
                    tokio::spawn(async move {
                        let _ = executor
                            .run(timeout, "sub_trade", move || b.subscribe_trade())
                            .await;
                    });
                }
            }
            P2Command::UnregisterTradeSubscription => {
                self.session.unregister_trade_subscription();
            }
            P2Command::RegisterBookSubscription { symbol } => {
                self.session.register_book_subscription(&symbol);
                if let Some(ref backend) = self.backend {
                    let b = Arc::clone(backend);
                    let sym = symbol.clone();
                    let executor = self.executor.clone();
                    let timeout = self.p2_timeout;
                    tokio::spawn(async move {
                        let _ = executor
                            .run(timeout, "sub_book", move || b.subscribe_book(&sym))
                            .await;
                    });
                }
            }
            P2Command::UnregisterBookSubscription { symbol } => {
                self.session.unregister_book_subscription(&symbol);
            }
            P2Command::RegisterBarsSubscription { symbol, timeframe } => {
                self.session.register_bars_subscription(&symbol, timeframe);
                if let Some(ref backend) = self.backend {
                    let b = Arc::clone(backend);
                    let sym = symbol.clone();
                    let executor = self.executor.clone();
                    let timeout = self.p2_timeout;
                    tokio::spawn(async move {
                        let _ = executor
                            .run(timeout, "sub_bars", move || {
                                b.subscribe_bars(&sym, timeframe)
                            })
                            .await;
                    });
                }
            }
            P2Command::UnregisterBarsSubscription { symbol, timeframe } => {
                self.session
                    .unregister_bars_subscription(&symbol, timeframe);
            }
        }
    }

    fn handle_p3(&mut self, cmd: P3Command) {
        match cmd {
            P3Command::CopyRates {
                symbol,
                timeframe,
                from,
                to,
                tx,
                cancel,
            } => {
                self.current_p3_cancel = Some(Arc::clone(&cancel));
                let Some(backend) = self.backend.clone() else {
                    let _ = tx.send(Err(Mt5Error::PipeDisconnected));
                    return;
                };
                let executor = self.executor.clone();
                let timeout = self.p3_timeout;
                let cancel_token = cancel;
                let dis_tx = self.disconnect_trigger_tx.clone();
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "copy_rates", move || {
                            backend.copy_rates_chunked(&symbol, timeframe, from, to, &cancel_token)
                        })
                        .await;
                    if matches!(
                        &res,
                        Err(Mt5Error::PipeDisconnected) | Err(Mt5Error::TransmissionFailed(_))
                    ) {
                        let _ = dis_tx.try_send(());
                    }
                    let _ = tx.send(res);
                });
            }
        }
    }

    fn handle_trade_event(&mut self, event: TradeEvent) {
        if event.event_seq > 0 {
            if self.last_event_seq > 0 && event.event_seq > self.last_event_seq + 1 {
                let gap = event.event_seq - self.last_event_seq - 1;
                warn!(
                    gap = gap,
                    last_seq = self.last_event_seq,
                    event_seq = event.event_seq,
                    "Detected TradeEvent event_seq gap! Triggering immediate broker reconciliation"
                );
                if let Some(backend) = self.backend.clone() {
                    let manager = self.order_manager.clone();
                    let book = Arc::clone(&self.position_book);
                    let executor = self.executor.clone();
                    let timeout = self.p2_timeout;
                    tokio::spawn(async move {
                        let res = executor
                            .run(timeout, "reconcile_on_gap", move || {
                                let res = manager.reconcile(&*backend);
                                if let Ok(ref report) = res {
                                    let _ = book
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .apply_snapshot(&report.positions);
                                }
                                res
                            })
                            .await;
                        if let Err(e) = res {
                            warn!("Reconciliation after event gap failed: {e:?}");
                        }
                    });
                }
            }
            self.last_event_seq = event.event_seq;
        }

        let _ = self
            .position_book
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .apply_trade_event(&event);
        let _ = self.order_manager.apply_trade_event(&event);

        // Fan out to all trade subscribers. broadcast::Sender::send returns the number of
        // receivers that accepted the event; if that is below receiver_count(), at least one
        // subscriber's buffer is full and that subscriber skipped this event (lag). Log the
        // lag so operators can detect it and trigger reconciliation if needed.
        let delivered = self.trade_event_out_tx.send(event);
        if let Ok(n) = delivered {
            let total = self.trade_event_out_tx.receiver_count();
            if n < total {
                warn!(
                    delivered = n,
                    subscribers = total,
                    skipped = total - n,
                    "Trade event fan-out lag: some subscribers skipped this event; call subscribe_trade_events().recv_checked() to detect lag on the consumer side"
                );
            }
        }
    }
}

impl OrderObserver for Mt5GatewayHandle {
    fn on_event(&self, event: &OrderEvent) {
        let _ = self.order_event_out_tx.send(event.clone());
    }
}

impl<B: TradingBackend + Send + Sync + 'static> OrderObserver for Mt5Gateway<B> {
    fn on_event(&self, event: &OrderEvent) {
        let _ = self.order_event_out_tx.send(event.clone());
    }
}

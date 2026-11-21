//! Dedicated MT5 Gateway actor with priority queues and event streaming.

use super::executor::BlockingExecutor;
use super::GatewayConfig;
use crate::backend::TradingBackend;
use crate::error::{Mt5Error, Result};
use crate::position_book::PositionBook;
use crate::reconciliation::{
    classify_send_error, LifecycleState, OrderManager, ReconciliationReport, SharedOrderManager,
};
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

    /// Ingest a trade transaction event into the lossless queue.
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

    /// Subscribe to the lossless broadcast stream of trade events.
    pub fn subscribe_trade_events(&self) -> broadcast::Receiver<TradeEvent> {
        self.trade_event_out_tx.subscribe()
    }

    /// Observe the current lifecycle state of the session.
    pub fn lifecycle_state(&self) -> LifecycleState {
        *self.state_watch.borrow()
    }

    /// Check if the session is ready for trading operations.
    pub fn is_ready(&self) -> bool {
        self.lifecycle_state() == LifecycleState::Ready
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
    backend: Arc<B>,
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
    p1_timeout: Duration,
    p2_timeout: Duration,
    p3_timeout: Duration,
}

impl<B: TradingBackend + Send + Sync + 'static> Mt5Gateway<B> {
    /// Spawn the MT5 Gateway actor on a dedicated Tokio task and return a client handle.
    pub fn spawn(
        backend: B,
        order_manager: OrderManager,
        session_config: SessionConfig,
    ) -> (Mt5GatewayHandle, tokio::task::JoinHandle<()>) {
        Self::spawn_with_config(
            backend,
            order_manager,
            GatewayConfig {
                session: session_config,
                ..Default::default()
            },
        )
    }

    /// Spawn the MT5 Gateway actor with full configuration options.
    pub fn spawn_with_config(
        backend: B,
        order_manager: OrderManager,
        config: GatewayConfig,
    ) -> (Mt5GatewayHandle, tokio::task::JoinHandle<()>) {
        let (p1_tx, p1_rx) = mpsc::channel(1024);
        let (p2_tx, p2_rx) = mpsc::channel(1024);
        let (p3_tx, p3_rx) = mpsc::channel(1024);
        let (trade_event_in_tx, trade_event_in_rx) = mpsc::unbounded_channel();
        let (trade_event_out_tx, _) = broadcast::channel(2048);
        let tick_watchers = Arc::new(Mutex::new(HashMap::new()));
        let (state_watch_tx, state_watch_rx) = watch::channel(LifecycleState::Ready);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let mut session = Mt5Session::new(config.session);
        session.set_state(LifecycleState::Ready);
        let strategy_magic = order_manager.strategy_magic();
        let position_book = Arc::new(Mutex::new(PositionBook::new(Some(strategy_magic))));
        let shared_order_manager = SharedOrderManager::new(order_manager);
        let executor = BlockingExecutor::new(config.executor_permits);

        let gateway = Self {
            backend: Arc::new(backend),
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
            p1_timeout: config.p1_timeout,
            p2_timeout: config.p2_timeout,
            p3_timeout: config.p3_timeout,
        };

        let handle = Mt5GatewayHandle {
            p1_tx,
            p2_tx,
            p3_tx,
            trade_event_in_tx,
            trade_event_out_tx,
            tick_watchers,
            state_watch: state_watch_rx,
            shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
        };

        let join_handle = tokio::spawn(async move {
            gateway.run().await;
        });

        (handle, join_handle)
    }

    /// Event loop processing priority queues with starvation-free scheduling.
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                biased;
                _ = &mut self.shutdown_rx => {
                    info!("Mt5Gateway received shutdown signal; stopping actor");
                    self.session.set_state(LifecycleState::Stopped);
                    let _ = self.state_watch.send(LifecycleState::Stopped);
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
                else => {
                    debug!("All Mt5Gateway channels closed; stopping actor");
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
        // High priority work preempts any in-flight bulk P3 download
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

                let manager = self.order_manager.clone();
                let backend = Arc::clone(&self.backend);
                let executor = self.executor.clone();
                let timeout = self.p1_timeout;
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p1_submit_order", move || {
                            manager.submit_order(&*backend, req)
                        })
                        .await;
                    let _ = tx.send(res);
                });
            }
            P1Command::ClosePosition { ticket, magic, tx } => {
                let manager = self.order_manager.clone();
                let backend = Arc::clone(&self.backend);
                let book = Arc::clone(&self.position_book);
                let executor = self.executor.clone();
                let timeout = self.p1_timeout;
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
                let manager = self.order_manager.clone();
                let backend = Arc::clone(&self.backend);
                let book = Arc::clone(&self.position_book);
                let executor = self.executor.clone();
                let timeout = self.p1_timeout;
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
                    let _ = tx.send(res);
                });
            }
        }
    }

    fn handle_p2(&mut self, cmd: P2Command) {
        match cmd {
            P2Command::AccountInfo { tx } => {
                let backend = Arc::clone(&self.backend);
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_account_info", move || backend.account_info())
                        .await;
                    let _ = tx.send(res);
                });
            }
            P2Command::GetPositions { magic, symbol, tx } => {
                let backend = Arc::clone(&self.backend);
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_get_positions", move || {
                            backend.positions_filtered(magic, symbol.as_deref())
                        })
                        .await;
                    let _ = tx.send(res);
                });
            }
            P2Command::GetPendingOrders { magic, symbol, tx } => {
                let backend = Arc::clone(&self.backend);
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_get_pending_orders", move || {
                            backend.pending_orders_filtered(magic, symbol.as_deref())
                        })
                        .await;
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
                let backend = Arc::clone(&self.backend);
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "p2_get_deals", move || {
                            backend.deals_filtered(from, to, magic, symbol.as_deref())
                        })
                        .await;
                    let _ = tx.send(res);
                });
            }
            P2Command::Reconcile { tx } => {
                let manager = self.order_manager.clone();
                let backend = Arc::clone(&self.backend);
                let book = Arc::clone(&self.position_book);
                let executor = self.executor.clone();
                let timeout = self.p2_timeout;
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
                let backend = Arc::clone(&self.backend);
                let executor = self.executor.clone();
                let timeout = self.p3_timeout;
                let cancel_token = cancel;
                tokio::spawn(async move {
                    let res = executor
                        .run(timeout, "copy_rates", move || {
                            backend.copy_rates_chunked(&symbol, timeframe, from, to, &cancel_token)
                        })
                        .await;
                    let _ = tx.send(res);
                });
            }
        }
    }

    fn handle_trade_event(&mut self, event: TradeEvent) {
        // Gap detection
        if event.event_seq > 0 {
            if self.last_event_seq > 0 && event.event_seq > self.last_event_seq + 1 {
                let gap = event.event_seq - self.last_event_seq - 1;
                warn!(
                    gap = gap,
                    last_seq = self.last_event_seq,
                    event_seq = event.event_seq,
                    "Detected TradeEvent event_seq gap! Triggering immediate broker reconciliation"
                );
                let manager = self.order_manager.clone();
                let backend = Arc::clone(&self.backend);
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
            self.last_event_seq = event.event_seq;
        }

        let _ = self
            .position_book
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .apply_trade_event(&event);
        let _ = self.order_manager.apply_trade_event(&event);
        let _ = self.trade_event_out_tx.send(event);
    }
}

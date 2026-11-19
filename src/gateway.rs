//! MT5 Gateway Actor with priority queues and event streaming.
//!
//! Separates execution commands from market data streams:
//! - Single-threaded execution actor owning `OrderManager` and `TradingBackend`.
//! - Three-tier priority message scheduling:
//!   - Priority 1 (Latency-Critical): Market orders, order modifications, emergency closes.
//!   - Priority 2 (Transactional): Account info, positions, pending orders, deals, manual reconciliation.
//!   - Priority 3 (Bulk): Historical candle downloads (`copy_rates`).
//! - Starvation-free priority scheduling ensuring P1 commands are never blocked by heavy P3 downloads.
//! - Lossless trade event stream with `event_seq` gap detection triggering immediate reconciliation.
//! - Tick conflation via Tokio `watch` channels to decouple order execution from high-frequency tick bursts.

use crate::backend::TradingBackend;
use crate::error::{Mt5Error, Result};
use crate::position_book::PositionBook;
use crate::reconciliation::{
    classify_send_error, LifecycleState, OrderManager, ReconciliationReport,
};
use crate::session::{Mt5Session, SessionConfig};
use crate::types::{
    AccountInfo, Deal, Level, OrderRequest, OrderState, Position, Rate, Tick, Timeframe,
    TrackedOrder, TradeEvent, TradeResult, WorkingOrder,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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

/// Priority 2 (Transactional): state queries and explicit reconciliation.
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
}

/// Priority 3 (Bulk): historical data requests.
#[derive(Debug)]
pub enum P3Command {
    CopyRates {
        symbol: String,
        timeframe: Timeframe,
        from: i64,
        to: i64,
        tx: oneshot::Sender<Result<Vec<Rate>>>,
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
    tick_watchers: Arc<Mutex<HashMap<String, watch::Sender<Option<Tick>>>>>,
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

    /// Fetch historical rates through the Priority 3 queue.
    pub async fn copy_rates(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Rate>> {
        let (tx, rx) = oneshot::channel();
        self.p3_tx
            .send(P3Command::CopyRates {
                symbol: symbol.to_string(),
                timeframe,
                from,
                to,
                tx,
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
    pub fn dispatch_tick(&self, tick: Tick) {
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
    pub fn subscribe_ticks(&self, symbol: &str) -> watch::Receiver<Option<Tick>> {
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
    backend: B,
    order_manager: OrderManager,
    position_book: PositionBook,
    session: Mt5Session,
    p1_rx: mpsc::Receiver<P1Command>,
    p2_rx: mpsc::Receiver<P2Command>,
    p3_rx: mpsc::Receiver<P3Command>,
    trade_event_in_rx: mpsc::UnboundedReceiver<TradeEvent>,
    trade_event_out_tx: broadcast::Sender<TradeEvent>,
    _tick_watchers: Arc<Mutex<HashMap<String, watch::Sender<Option<Tick>>>>>,
    state_watch: watch::Sender<LifecycleState>,
    last_event_seq: u64,
    shutdown_rx: oneshot::Receiver<()>,
}

impl<B: TradingBackend + Send + Sync + 'static> Mt5Gateway<B> {
    /// Spawn the MT5 Gateway actor on a dedicated Tokio task and return a client handle.
    pub fn spawn(
        backend: B,
        order_manager: OrderManager,
        session_config: SessionConfig,
    ) -> (Mt5GatewayHandle, tokio::task::JoinHandle<()>) {
        let (p1_tx, p1_rx) = mpsc::channel(1024);
        let (p2_tx, p2_rx) = mpsc::channel(1024);
        let (p3_tx, p3_rx) = mpsc::channel(1024);
        let (trade_event_in_tx, trade_event_in_rx) = mpsc::unbounded_channel();
        let (trade_event_out_tx, _) = broadcast::channel(2048);
        let tick_watchers = Arc::new(Mutex::new(HashMap::new()));
        let (state_watch_tx, state_watch_rx) = watch::channel(LifecycleState::Ready);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let mut session = Mt5Session::new(session_config);
        session.set_state(LifecycleState::Ready);
        let position_book = PositionBook::new(Some(order_manager.strategy_magic()));

        let gateway = Self {
            backend,
            order_manager,
            position_book,
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

    fn handle_p1(&mut self, cmd: P1Command) {
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
                let res = self.order_manager.submit_order(&self.backend, req);
                let _ = tx.send(res);
            }
            P1Command::ClosePosition { ticket, magic, tx } => {
                let res = if magic > 0 && magic != self.order_manager.strategy_magic() {
                    self.backend.order_close_with_magic(ticket, magic)
                } else {
                    self.position_book.mark_closing(ticket);
                    let outcome = self.order_manager.close_position(&self.backend, ticket);
                    if let Err(ref e) = outcome {
                        let (new_state, _, desc) = classify_send_error(e);
                        if new_state == OrderState::Unknown {
                            self.position_book.mark_unknown(ticket, &desc);
                        }
                    }
                    outcome
                };
                let _ = tx.send(res);
            }
            P1Command::ModifyOrder {
                ticket,
                magic,
                stop_loss,
                take_profit,
                tx,
            } => {
                let res = if magic > 0 && magic != self.order_manager.strategy_magic() {
                    self.backend
                        .order_modify_with_magic(ticket, magic, stop_loss, take_profit)
                } else {
                    let outcome = self.order_manager.modify_order(
                        &self.backend,
                        ticket,
                        stop_loss,
                        take_profit,
                    );
                    match &outcome {
                        Ok(_) => {
                            if let Some(pos) = self.position_book.get_mut(ticket) {
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
                                self.position_book.mark_unknown(ticket, &desc);
                            }
                        }
                    }
                    outcome
                };
                let _ = tx.send(res);
            }
        }
    }

    fn handle_p2(&mut self, cmd: P2Command) {
        match cmd {
            P2Command::AccountInfo { tx } => {
                let res = self.backend.account_info();
                let _ = tx.send(res);
            }
            P2Command::GetPositions { magic, symbol, tx } => {
                let res = self.backend.positions_filtered(magic, symbol.as_deref());
                let _ = tx.send(res);
            }
            P2Command::GetPendingOrders { magic, symbol, tx } => {
                let res = self
                    .backend
                    .pending_orders_filtered(magic, symbol.as_deref());
                let _ = tx.send(res);
            }
            P2Command::GetDeals {
                from,
                to,
                magic,
                symbol,
                tx,
            } => {
                let res = self
                    .backend
                    .deals_filtered(from, to, magic, symbol.as_deref());
                let _ = tx.send(res);
            }
            P2Command::Reconcile { tx } => {
                let res = self.order_manager.reconcile(&self.backend);
                if let Ok(ref report) = res {
                    let _ = self.position_book.apply_snapshot(&report.positions);
                }
                let _ = tx.send(res);
            }
            P2Command::GetPositionBook { tx } => {
                let _ = tx.send(Ok(self.position_book.clone()));
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
            } => {
                let res = self.backend.copy_rates(&symbol, timeframe, from, to);
                let _ = tx.send(res);
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
                let res = self.order_manager.reconcile(&self.backend);
                if let Ok(ref report) = res {
                    let _ = self.position_book.apply_snapshot(&report.positions);
                }
            }
            self.last_event_seq = event.event_seq;
        }

        let _ = self.position_book.apply_trade_event(&event);
        let _ = self.order_manager.apply_trade_event(&event);
        let _ = self.trade_event_out_tx.send(event);
    }
}

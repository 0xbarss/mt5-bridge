//! MT5 Session Supervisor and Reconnect Management.
//!
//! Provides:
//! - Automated connection monitoring and disconnect detection.
//! - Exponential backoff ladder with configurable factors and caps.
//! - Strict lifecycle state tracking (`LifecycleState`: `Starting` -> `Connecting` -> `Authenticating` -> `Syncing` -> `Reconciling` -> `Ready` / `Degraded` / `Stopped`).
//! - Automatic tracking and restoration of symbol tick, book, and trade event subscriptions upon reconnection.
//! - Mandatory broker state reconciliation before unlocking order submissions.

#[cfg(feature = "async")]
use crate::error::{Mt5Error, Result};
use crate::reconciliation::LifecycleState;
use crate::types::Timeframe;
use std::collections::HashSet;
use std::time::Duration;
#[cfg(feature = "async")]
use tokio::sync::watch;
use tracing::{debug, info, warn};

/// Configuration governing session reconnection, backoff, and health checks.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionConfig {
    /// Initial backoff delay for the first reconnection attempt.
    pub initial_backoff: Duration,
    /// Maximum backoff cap for reconnection retries.
    pub max_backoff: Duration,
    /// Multiplicative factor for exponential backoff progression.
    pub backoff_factor: f64,
    /// Maximum reconnection attempts before entering terminal `Stopped` state. `None` = unlimited.
    pub max_reconnect_attempts: Option<u32>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
            backoff_factor: 2.0,
            max_reconnect_attempts: None,
        }
    }
}

/// Supervised session state machine tracking connectivity, subscriptions, and reconciliation.
#[derive(Debug)]
pub struct Mt5Session {
    state: LifecycleState,
    config: SessionConfig,
    consecutive_failures: u32,
    current_backoff: Duration,
    subscribed_ticks: HashSet<String>,
    subscribed_trades: bool,
    subscribed_books: HashSet<String>,
    subscribed_bars: HashSet<(String, Timeframe)>,
    #[cfg(feature = "async")]
    state_tx: watch::Sender<LifecycleState>,
}

impl Mt5Session {
    /// Create a new session initialized in the `Starting` state.
    pub fn new(config: SessionConfig) -> Self {
        let initial_backoff = config.initial_backoff;
        #[cfg(feature = "async")]
        let (state_tx, _) = watch::channel(LifecycleState::Starting);
        Self {
            state: LifecycleState::Starting,
            config,
            consecutive_failures: 0,
            current_backoff: initial_backoff,
            subscribed_ticks: HashSet::new(),
            subscribed_trades: false,
            subscribed_books: HashSet::new(),
            subscribed_bars: HashSet::new(),
            #[cfg(feature = "async")]
            state_tx,
        }
    }

    #[inline]
    fn notify_state(&self, _new_state: LifecycleState) {
        #[cfg(feature = "async")]
        let _ = self.state_tx.send_replace(_new_state);
    }

    /// Current operational lifecycle state.
    pub fn state(&self) -> LifecycleState {
        self.state
    }

    /// Subscribe to changes in the lifecycle state stream.
    #[cfg(feature = "async")]
    pub fn state_stream(&self) -> watch::Receiver<LifecycleState> {
        self.state_tx.subscribe()
    }

    /// Wait until the session transitions to `LifecycleState::Ready`, timing out after `timeout`.
    #[cfg(feature = "async")]
    pub async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        tokio::time::timeout(timeout, async {
            let mut rx = self.state_tx.subscribe();
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

    /// Whether the session is reconciled and ready to safely accept trade operations.
    pub fn is_ready(&self) -> bool {
        self.state == LifecycleState::Ready
    }

    /// Number of consecutive connection failures observed.
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Current backoff delay before the next retry attempt.
    pub fn current_backoff(&self) -> Duration {
        self.current_backoff
    }

    /// Set lifecycle state explicitly.
    pub fn set_state(&mut self, new_state: LifecycleState) {
        if self.state != new_state {
            debug!(
                from = ?self.state,
                to = ?new_state,
                "Mt5Session state transition"
            );
            self.state = new_state;
            self.notify_state(new_state);
        }
    }

    /// Signal a connection loss or communication failure.
    ///
    /// Transitions state to `Degraded`, increments failure counter, and advances the backoff ladder.
    pub fn on_disconnect(&mut self, reason: &str) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.state = LifecycleState::Degraded;

        if let Some(max_attempts) = self.config.max_reconnect_attempts {
            if self.consecutive_failures > max_attempts {
                warn!(
                    failures = self.consecutive_failures,
                    max = max_attempts,
                    "Max reconnection attempts exceeded; stopping session"
                );
                self.state = LifecycleState::Stopped;
                self.notify_state(LifecycleState::Stopped);
                return;
            }
        }

        // Calculate next exponential backoff: min(initial * factor^(failures - 1), max_backoff)
        let exponent = (self.consecutive_failures - 1).min(16) as i32;
        let multiplier = self.config.backoff_factor.powi(exponent);
        let calculated = self.config.initial_backoff.mul_f64(multiplier);
        self.current_backoff = calculated.min(self.config.max_backoff);

        self.notify_state(self.state);

        warn!(
            failures = self.consecutive_failures,
            backoff_ms = self.current_backoff.as_millis(),
            reason = reason,
            "Mt5Session disconnected; state transitioned to Degraded"
        );
    }

    /// Signal a successful connection establishment.
    ///
    /// Transitions state to `Syncing` and resets consecutive failure counters.
    pub fn on_connected(&mut self) {
        info!("Mt5Session connected; transitioning to Syncing");
        self.consecutive_failures = 0;
        self.current_backoff = self.config.initial_backoff;
        self.state = LifecycleState::Syncing;
        self.notify_state(self.state);
    }

    /// Signal successful completion of symbol syncing, ready to begin mandatory reconciliation.
    pub fn on_synced(&mut self) {
        info!("Mt5Session synced; transitioning to Reconciling");
        self.state = LifecycleState::Reconciling;
        self.notify_state(self.state);
    }

    /// Signal successful completion of broker reconciliation.
    ///
    /// Transitions state to `Ready`, safely unlocking trade execution.
    pub fn on_reconciled(&mut self) {
        info!("Mt5Session reconciled successfully; transitioning to Ready");
        self.state = LifecycleState::Ready;
        self.notify_state(self.state);
    }

    /// Register a symbol tick subscription to be preserved across reconnects.
    pub fn register_tick_subscription(&mut self, symbol: &str) {
        self.subscribed_ticks.insert(symbol.to_string());
    }

    /// Unregister a symbol tick subscription.
    pub fn unregister_tick_subscription(&mut self, symbol: &str) {
        self.subscribed_ticks.remove(symbol);
    }

    /// Register trade event subscription to be preserved across reconnects.
    pub fn register_trade_subscription(&mut self) {
        self.subscribed_trades = true;
    }

    /// Unregister trade event subscription.
    pub fn unregister_trade_subscription(&mut self) {
        self.subscribed_trades = false;
    }

    /// Register depth-of-market book subscription.
    pub fn register_book_subscription(&mut self, symbol: &str) {
        self.subscribed_books.insert(symbol.to_string());
    }

    /// Unregister depth-of-market book subscription.
    pub fn unregister_book_subscription(&mut self, symbol: &str) {
        self.subscribed_books.remove(symbol);
    }

    /// Set of symbols currently registered for tick feeds.
    pub fn active_tick_subscriptions(&self) -> &HashSet<String> {
        &self.subscribed_ticks
    }

    /// Whether trade transaction streaming is registered.
    pub fn is_trade_subscribed(&self) -> bool {
        self.subscribed_trades
    }

    /// Set of symbols currently registered for book feeds.
    pub fn active_book_subscriptions(&self) -> &HashSet<String> {
        &self.subscribed_books
    }

    /// Register closed-bar subscription.
    pub fn register_bars_subscription(&mut self, symbol: &str, timeframe: Timeframe) {
        self.subscribed_bars.insert((symbol.to_string(), timeframe));
    }

    /// Unregister closed-bar subscription.
    pub fn unregister_bars_subscription(&mut self, symbol: &str, timeframe: Timeframe) {
        self.subscribed_bars
            .remove(&(symbol.to_string(), timeframe));
    }

    /// Set of (symbol, timeframe) currently registered for bar feeds.
    pub fn active_bar_subscriptions(&self) -> &HashSet<(String, Timeframe)> {
        &self.subscribed_bars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_lifecycle_progression() {
        let mut session = Mt5Session::new(SessionConfig::default());
        assert_eq!(session.state(), LifecycleState::Starting);
        assert!(!session.is_ready());

        session.on_connected();
        assert_eq!(session.state(), LifecycleState::Syncing);
        assert!(!session.is_ready());

        session.on_synced();
        assert_eq!(session.state(), LifecycleState::Reconciling);
        assert!(!session.is_ready());

        session.on_reconciled();
        assert_eq!(session.state(), LifecycleState::Ready);
        assert!(session.is_ready());
    }

    #[test]
    fn test_session_backoff_ladder_and_reset() {
        let config = SessionConfig {
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_millis(500),
            backoff_factor: 2.0,
            max_reconnect_attempts: Some(4),
        };
        let mut session = Mt5Session::new(config);

        assert_eq!(session.consecutive_failures(), 0);
        assert_eq!(session.current_backoff(), Duration::from_millis(50));

        // Failure 1: 50 * 2^0 = 50ms
        session.on_disconnect("pipe closed");
        assert_eq!(session.state(), LifecycleState::Degraded);
        assert_eq!(session.consecutive_failures(), 1);
        assert_eq!(session.current_backoff(), Duration::from_millis(50));

        // Failure 2: 50 * 2^1 = 100ms
        session.on_disconnect("pipe timeout");
        assert_eq!(session.consecutive_failures(), 2);
        assert_eq!(session.current_backoff(), Duration::from_millis(100));

        // Failure 3: 50 * 2^2 = 200ms
        session.on_disconnect("connect refused");
        assert_eq!(session.consecutive_failures(), 3);
        assert_eq!(session.current_backoff(), Duration::from_millis(200));

        // Failure 4: 50 * 2^3 = 400ms
        session.on_disconnect("connect refused");
        assert_eq!(session.consecutive_failures(), 4);
        assert_eq!(session.current_backoff(), Duration::from_millis(400));

        // Failure 5: exceeded max_reconnect_attempts (4) -> Stopped
        session.on_disconnect("connect refused");
        assert_eq!(session.consecutive_failures(), 5);
        assert_eq!(session.state(), LifecycleState::Stopped);

        // Connected resets state
        session.on_connected();
        assert_eq!(session.consecutive_failures(), 0);
        assert_eq!(session.current_backoff(), Duration::from_millis(50));
        assert_eq!(session.state(), LifecycleState::Syncing);
    }

    #[test]
    fn test_session_subscriptions_tracking() {
        let mut session = Mt5Session::new(SessionConfig::default());

        session.register_tick_subscription("EURUSD");
        session.register_tick_subscription("GBPUSD");
        assert_eq!(session.active_tick_subscriptions().len(), 2);
        assert!(session.active_tick_subscriptions().contains("EURUSD"));

        session.unregister_tick_subscription("EURUSD");
        assert_eq!(session.active_tick_subscriptions().len(), 1);
        assert!(!session.active_tick_subscriptions().contains("EURUSD"));
        assert!(session.active_tick_subscriptions().contains("GBPUSD"));

        assert!(!session.is_trade_subscribed());
        session.register_trade_subscription();
        assert!(session.is_trade_subscribed());
        session.unregister_trade_subscription();
        assert!(!session.is_trade_subscribed());

        session.register_book_subscription("USDJPY");
        assert_eq!(session.active_book_subscriptions().len(), 1);
        assert!(session.active_book_subscriptions().contains("USDJPY"));
        session.unregister_book_subscription("USDJPY");
        assert_eq!(session.active_book_subscriptions().len(), 0);
    }
}

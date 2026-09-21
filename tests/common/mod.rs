//! Fault-injecting simulated broker + EA used by the failure-injection tests.
//!
//! `MockBroker` implements [`TradingBackend`], so it can drive `OrderManager` exactly like the
//! real `Mt5Client`, but it can be told to misbehave on demand: drop a response *after* the
//! order executed, drop the request *before* it executed, delay position visibility, restart
//! the EA (wiping its idempotency cache), fill partially, switch between hedging/netting, etc.
//!
//! It emulates the EA's idempotency cache using the **fixed** rule implemented in
//! `mql5/Experts/mt5_bridge.mq5` (`ExtractClientOrderId`): only a well-formed
//! `cid:<13 Crockford-base32 chars>` prefix is an idempotency key, free-text comments never are,
//! and a cache hit whose symbol/type/volume differ is rejected rather than replayed.
#![allow(dead_code)]

use mt5_bridge::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

pub const MAGIC: u64 = 424242;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fault {
    /// Request never leaves the process (`TransmissionFailed`); nothing executes.
    TransmissionFailure,
    /// The order **executes** on the broker but the response is lost → `UnknownExecutionState`.
    ExecuteThenLoseResponse,
    /// The request is lost before the EA processes it → `UnknownExecutionState`; nothing executes.
    LoseBeforeExecution,
    /// The broker rejects the request with this MT5 retcode.
    Reject(u32),
    /// Market order fills only this volume (retcode 10010).
    PartialFill(f64),
    /// Order executes, but only this volume fills, and the response is then lost.
    PartialFillThenLoseResponse(f64),
}

#[derive(Clone)]
struct CachedExec {
    symbol: String,
    order_type: OrderType,
    volume: f64,
    result: TradeResult,
}

struct Inner {
    next_ticket: u64,
    positions: Vec<(Position, u64)>, // (position, visible_from_query)
    pending: Vec<(WorkingOrder, u64)>,
    deals: Vec<(Deal, u64)>,
    ea_cache: HashMap<String, CachedExec>,
    ea_idempotency: bool,
    faults: VecDeque<Fault>,
    executions: usize,
    send_calls: usize,
    query_count: u64,
    visibility_delay: u64,
    history_supported: bool,
    netting: bool,
    fill_price: f64,
}

pub struct MockBroker {
    inner: Mutex<Inner>,
}

/// The EA's idempotency-key extraction rule (mirrors `ExtractClientOrderId` in the .mq5).
pub fn ea_extract_key(comment: &str) -> Option<String> {
    const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let rest = comment.strip_prefix("cid:")?;
    let token = rest.get(..13)?;
    if !token.chars().all(|c| ALPHABET.contains(c)) {
        return None;
    }
    match rest[13..].chars().next() {
        None | Some(':') => Some(token.to_string()),
        Some(_) => None,
    }
}

impl MockBroker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next_ticket: 1000,
                positions: vec![],
                pending: vec![],
                deals: vec![],
                ea_cache: HashMap::new(),
                ea_idempotency: true,
                faults: VecDeque::new(),
                executions: 0,
                send_calls: 0,
                query_count: 0,
                visibility_delay: 0,
                history_supported: true,
                netting: false,
                fill_price: 1.0850,
            }),
        }
    }

    // ---- configuration / fault injection ----
    pub fn push_fault(&self, f: Fault) {
        self.inner.lock().unwrap().faults.push_back(f);
    }
    /// Newly executed positions/deals only become visible after this many further queries.
    pub fn set_visibility_delay(&self, queries: u64) {
        self.inner.lock().unwrap().visibility_delay = queries;
    }
    pub fn set_history_supported(&self, yes: bool) {
        self.inner.lock().unwrap().history_supported = yes;
    }
    pub fn set_ea_idempotency(&self, yes: bool) {
        self.inner.lock().unwrap().ea_idempotency = yes;
    }
    pub fn set_netting(&self, yes: bool) {
        self.inner.lock().unwrap().netting = yes;
    }
    /// EA / MT5 restart: the EA's in-memory idempotency cache is lost; broker state persists.
    pub fn restart_ea(&self) {
        self.inner.lock().unwrap().ea_cache.clear();
    }

    // ---- observation ----
    /// How many orders *actually executed* on the broker (the number that matters).
    pub fn executions(&self) -> usize {
        self.inner.lock().unwrap().executions
    }
    pub fn send_calls(&self) -> usize {
        self.inner.lock().unwrap().send_calls
    }
    pub fn all_positions(&self) -> Vec<Position> {
        self.inner
            .lock()
            .unwrap()
            .positions
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }

    // ---- direct state injection (for scenarios that pre-date the manager) ----
    pub fn inject_position(&self, p: Position) {
        self.inner.lock().unwrap().positions.push((p, 0));
    }
    pub fn inject_pending(&self, o: WorkingOrder) {
        self.inner.lock().unwrap().pending.push((o, 0));
    }
    pub fn inject_deal(&self, d: Deal) {
        self.inner.lock().unwrap().deals.push((d, 0));
    }
    pub fn close_externally(&self, position_ticket: u64) {
        let mut g = self.inner.lock().unwrap();
        if let Some(i) = g
            .positions
            .iter()
            .position(|(p, _)| p.ticket == position_ticket)
        {
            let (p, _) = g.positions.remove(i);
            let t = g.next_ticket;
            g.next_ticket += 1;
            let d = Deal {
                ticket: t,
                order: t,
                position_id: p.ticket,
                time: chrono::Utc::now().timestamp(),
                direction: if p.is_buy() {
                    OrderType::Sell
                } else {
                    OrderType::Buy
                },
                entry: DealEntry::Out,
                magic: p.magic,
                volume: p.volume,
                price: g.fill_price,
                commission: 0.0,
                swap: 0.0,
                profit: 0.0,
                symbol: p.symbol.clone(),
                comment: "[sl]".to_string(),
            };
            g.deals.push((d, 0));
        }
    }
}

impl Inner {
    fn new_ticket(&mut self) -> u64 {
        let t = self.next_ticket;
        self.next_ticket += 1;
        t
    }

    /// Emulates the EA + broker executing a request. Returns the synchronous result.
    fn execute(&mut self, req: &OrderRequest, fill_override: Option<f64>) -> Result<TradeResult> {
        let comment = req.effective_comment();
        let key = if self.ea_idempotency {
            ea_extract_key(&comment)
        } else {
            None
        };

        if let Some(k) = &key {
            if let Some(c) = self.ea_cache.get(k) {
                let same = c.symbol == req.symbol
                    && c.order_type == req.order_type
                    && (c.volume - req.volume).abs() < 1e-9;
                if !same {
                    // Fixed EA behaviour: a key collision with a *different* request is an
                    // error, never a silent replay of someone else's execution.
                    return Err(Mt5Error::OrderSendFailed {
                        symbol: req.symbol.clone(),
                        retcode: 10013,
                        description: mt5_retcode_description(10013),
                    });
                }
                return Ok(c.result);
            }
        }

        let magic = req.magic.unwrap_or(MAGIC);
        let visible_from = self.query_count + 1 + self.visibility_delay;
        let now = chrono::Utc::now().timestamp();
        let is_market = matches!(req.order_type, OrderType::Buy | OrderType::Sell);
        self.executions += 1;

        let result = if is_market {
            let vol = fill_override.unwrap_or(req.volume);
            let order_ticket = self.new_ticket();
            let deal_ticket = self.new_ticket();
            let price = self.fill_price;

            // netting: merge into an existing same-symbol position; hedging: new position
            let merge_idx = if self.netting {
                self.positions
                    .iter()
                    .position(|(p, _)| p.symbol == req.symbol && p.magic == magic)
            } else {
                None
            };
            let pos_ticket = match merge_idx {
                Some(i) => {
                    let p = &mut self.positions[i].0;
                    p.volume += vol;
                    p.ticket
                }
                None => {
                    let t = self.new_ticket();
                    self.positions.push((
                        Position {
                            ticket: t,
                            time: now,
                            position_type: req.order_type,
                            magic,
                            volume: vol,
                            price_open: price,
                            stop_loss: req.stop_loss,
                            take_profit: req.take_profit,
                            price_current: price,
                            profit: 0.0,
                            swap: 0.0,
                            symbol: req.symbol.clone(),
                            comment: comment.clone(),
                        },
                        visible_from,
                    ));
                    t
                }
            };
            self.deals.push((
                Deal {
                    ticket: deal_ticket,
                    order: order_ticket,
                    position_id: pos_ticket,
                    time: now,
                    direction: req.order_type,
                    entry: DealEntry::In,
                    magic,
                    volume: vol,
                    price,
                    commission: 0.0,
                    swap: 0.0,
                    profit: 0.0,
                    symbol: req.symbol.clone(),
                    comment: comment.clone(),
                },
                visible_from,
            ));
            let partial = fill_override.is_some_and(|v| v + 1e-12 < req.volume);
            TradeResult {
                retcode: if partial { 10010 } else { 10009 },
                deal: deal_ticket,
                order: order_ticket,
                position: pos_ticket,
                volume: vol,
                price,
            }
        } else {
            let order_ticket = self.new_ticket();
            self.pending.push((
                WorkingOrder {
                    ticket: order_ticket,
                    time_setup: now,
                    order_type: req.order_type,
                    magic,
                    volume_initial: req.volume,
                    volume_current: req.volume,
                    price_open: req.price,
                    stop_loss: req.stop_loss,
                    take_profit: req.take_profit,
                    price_current: self.fill_price,
                    symbol: req.symbol.clone(),
                    comment: comment.clone(),
                },
                visible_from,
            ));
            TradeResult {
                retcode: 10008,
                deal: 0,
                order: order_ticket,
                position: 0,
                volume: req.volume,
                price: req.price,
            }
        };

        if let Some(k) = key {
            self.ea_cache.insert(
                k,
                CachedExec {
                    symbol: req.symbol.clone(),
                    order_type: req.order_type,
                    volume: req.volume,
                    result,
                },
            );
        }
        Ok(result)
    }
}

fn unknown(req: &OrderRequest) -> Mt5Error {
    Mt5Error::UnknownExecutionState {
        symbol: req.symbol.clone(),
        client_order_id: req.client_order_id.clone(),
        description: "injected: pipe response lost".to_string(),
    }
}

impl TradingBackend for MockBroker {
    fn order_send(&self, req: &OrderRequest) -> Result<TradeResult> {
        req.validate()?;
        let mut g = self.inner.lock().unwrap();
        g.send_calls += 1;
        match g.faults.pop_front() {
            Some(Fault::TransmissionFailure) => Err(Mt5Error::TransmissionFailed(
                "injected: send failed before transmission".to_string(),
            )),
            Some(Fault::LoseBeforeExecution) => Err(unknown(req)),
            Some(Fault::Reject(rc)) => Err(Mt5Error::OrderSendFailed {
                symbol: req.symbol.clone(),
                retcode: rc,
                description: mt5_retcode_description(rc),
            }),
            Some(Fault::ExecuteThenLoseResponse) => {
                let _ = g.execute(req, None)?;
                Err(unknown(req))
            }
            Some(Fault::PartialFillThenLoseResponse(v)) => {
                let _ = g.execute(req, Some(v))?;
                Err(unknown(req))
            }
            Some(Fault::PartialFill(v)) => g.execute(req, Some(v)),
            None => g.execute(req, None),
        }
    }

    fn order_close_with_magic(&self, ticket: u64, magic: u64) -> Result<TradeResult> {
        let g = self.inner.lock().unwrap();
        let idx = g
            .positions
            .iter()
            .position(|(p, _)| p.ticket == ticket)
            .ok_or(Mt5Error::OrderCloseFailed {
                ticket,
                retcode: 10036,
                description: mt5_retcode_description(10036),
            })?;
        let actual = g.positions[idx].0.magic;
        if magic != 0 && actual != magic {
            return Err(Mt5Error::OwnershipMismatch {
                ticket,
                expected_magic: magic,
                actual_magic: actual,
            });
        }
        drop(g);
        self.close_externally(ticket);
        Ok(TradeResult {
            retcode: 10009,
            deal: 0,
            order: 0,
            position: ticket,
            volume: 0.0,
            price: 0.0,
        })
    }

    fn order_modify_with_magic(
        &self,
        ticket: u64,
        magic: u64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<TradeResult> {
        let mut g = self.inner.lock().unwrap();
        for (p, _) in g.positions.iter_mut() {
            if p.ticket == ticket {
                if magic != 0 && p.magic != magic {
                    return Err(Mt5Error::OwnershipMismatch {
                        ticket,
                        expected_magic: magic,
                        actual_magic: p.magic,
                    });
                }
                p.stop_loss = stop_loss;
                p.take_profit = take_profit;
                return Ok(TradeResult {
                    retcode: 10009,
                    deal: 0,
                    order: 0,
                    position: ticket,
                    volume: 0.0,
                    price: 0.0,
                });
            }
        }
        Err(Mt5Error::OrderModifyFailed {
            ticket,
            retcode: 10036,
            description: mt5_retcode_description(10036),
        })
    }

    fn positions_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Position>> {
        let mut g = self.inner.lock().unwrap();
        g.query_count += 1;
        let q = g.query_count;
        Ok(g.positions
            .iter()
            .filter(|(_, vis)| *vis <= q)
            .map(|(p, _)| p)
            .filter(|p| magic.is_none_or(|m| p.magic == m))
            .filter(|p| symbol.is_none_or(|s| p.symbol == s))
            .cloned()
            .collect())
    }

    fn pending_orders_filtered(
        &self,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<WorkingOrder>> {
        let g = self.inner.lock().unwrap();
        let q = g.query_count;
        Ok(g.pending
            .iter()
            .filter(|(_, vis)| *vis <= q)
            .map(|(o, _)| o)
            .filter(|o| magic.is_none_or(|m| o.magic == m))
            .filter(|o| symbol.is_none_or(|s| o.symbol == s))
            .cloned()
            .collect())
    }

    fn deals_filtered(
        &self,
        from: i64,
        to: i64,
        magic: Option<u64>,
        symbol: Option<&str>,
    ) -> Result<Vec<Deal>> {
        let g = self.inner.lock().unwrap();
        if !g.history_supported {
            return Err(Mt5Error::UnsupportedFeature("DealsGet"));
        }
        let q = g.query_count;
        Ok(g.deals
            .iter()
            .filter(|(_, vis)| *vis <= q)
            .map(|(d, _)| d)
            .filter(|d| d.time >= from && d.time <= to)
            .filter(|d| magic.is_none_or(|m| d.magic == m))
            .filter(|d| symbol.is_none_or(|s| d.symbol == s))
            .cloned()
            .collect())
    }
}

// ---- helpers shared by tests ----

/// A policy that decides on the first pass (for tests not about the grace period).
pub fn instant_policy() -> SafetyPolicy {
    SafetyPolicy {
        min_absent_observations: 1,
        min_absent_secs: 0,
        ..Default::default()
    }
}

/// Grace-period-free policy but otherwise default (block scope = Strategy, history required).
pub fn manager() -> OrderManager {
    OrderManager::with_policy(MAGIC, instant_policy())
}

pub fn buy(symbol: &str, vol: f64, cid: &str) -> OrderRequest {
    OrderRequest::buy(symbol, vol).client_order_id(cid)
}

/// Unique scratch directory under the OS temp dir (no external crate needed).
pub fn scratch_dir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mt5bridge-test-{}-{}-{}",
        tag,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

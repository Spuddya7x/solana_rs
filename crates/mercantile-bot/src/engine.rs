//! The tick loop that ties everything together.
//!
//! One tick is: refresh every pool in the universe, show each market to each
//! strategy, pass the resulting signals through the risk manager, execute what
//! survives, and journal all of it. Nothing in here knows whether execution is
//! paper or live.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use mercantile_core::Market;
use mercantile_dex::client::PoolFetch;
use mercantile_dex::pool::PoolState;

use crate::execution::Executor;
use crate::journal::{Event, Journal};
use crate::market::{MarketHistories, MarketView};
use crate::portfolio::Portfolio;
use crate::risk::{Decision, RiskManager};
use crate::strategy::Strategy;

/// What one tick did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TickSummary {
    pub markets_priced: usize,
    pub markets_unavailable: usize,
    pub signals: usize,
    pub approved: usize,
    pub rejected: usize,
    pub fills: usize,
    pub errors: usize,
}

/// Live balances, refreshed each tick in live mode.
pub trait BalanceSource {
    /// GP balance in whole GP, and item counts keyed by registry key.
    fn balances(&self, markets: &[Market]) -> Result<(f64, BTreeMap<String, u64>)>;
    /// SOL balance, if it is knowable.
    fn sol_balance(&self) -> Result<Option<f64>>;
}

/// Nothing to sync — paper mode keeps its own book.
pub struct NoBalances;

impl BalanceSource for NoBalances {
    fn balances(&self, _markets: &[Market]) -> Result<(f64, BTreeMap<String, u64>)> {
        Ok((f64::NAN, BTreeMap::new()))
    }
    fn sol_balance(&self) -> Result<Option<f64>> {
        Ok(None)
    }
}

/// The bot.
pub struct Engine<F, E, B> {
    markets: Vec<Market>,
    fetcher: F,
    executor: E,
    balances: B,
    strategies: Vec<Box<dyn Strategy>>,
    risk: RiskManager,
    portfolio: Portfolio,
    histories: MarketHistories,
    journal: Journal,
    record_snapshots: bool,
    sync_balances: bool,
    /// Consecutive fetch failures; the engine gives up rather than spinning.
    consecutive_failures: u32,
}

/// How many ticks in a row may fail before the engine stops.
const MAX_CONSECUTIVE_FAILURES: u32 = 10;

impl<F: PoolFetch, E: Executor, B: BalanceSource> Engine<F, E, B> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        markets: Vec<Market>,
        fetcher: F,
        executor: E,
        balances: B,
        strategies: Vec<Box<dyn Strategy>>,
        risk: RiskManager,
        portfolio: Portfolio,
        history_capacity: usize,
        journal: Journal,
    ) -> Self {
        Self {
            markets,
            fetcher,
            executor,
            balances,
            strategies,
            risk,
            portfolio,
            histories: MarketHistories::new(history_capacity),
            journal,
            record_snapshots: false,
            sync_balances: false,
            consecutive_failures: 0,
        }
    }

    /// Write a price snapshot per market per tick.
    pub fn with_snapshots(mut self, record: bool) -> Self {
        self.record_snapshots = record;
        self
    }

    /// Refresh balances from the chain each tick (live mode).
    pub fn with_balance_sync(mut self, sync: bool) -> Self {
        self.sync_balances = sync;
        self
    }

    pub fn portfolio(&self) -> &Portfolio {
        &self.portfolio
    }

    pub fn markets(&self) -> &[Market] {
        &self.markets
    }

    /// Run until `shutdown` is set, or until too many ticks fail in a row.
    pub fn run(&mut self, interval: Duration, shutdown: Arc<AtomicBool>) -> Result<()> {
        self.journal.record(&Event::Start {
            ts: now(),
            mode: self.executor.mode().to_string(),
            markets: self.markets.len(),
            strategies: self
                .strategies
                .iter()
                .map(|s| s.name().to_string())
                .collect(),
            gp: self.portfolio.gp,
        });

        let stop_reason = loop {
            if shutdown.load(Ordering::Relaxed) {
                break "interrupted".to_string();
            }
            let started = Instant::now();
            match self.tick() {
                Ok(summary) => {
                    tracing::info!(
                        priced = summary.markets_priced,
                        signals = summary.signals,
                        fills = summary.fills,
                        rejected = summary.rejected,
                        gp = format_args!("{:.2}", self.portfolio.gp),
                        pnl = format_args!("{:.2}", self.portfolio.realized_pnl),
                        "tick"
                    );
                }
                Err(err) => {
                    self.consecutive_failures += 1;
                    tracing::error!(%err, attempt = self.consecutive_failures, "tick failed");
                    self.journal.record(&Event::Error {
                        ts: now(),
                        context: "tick".to_string(),
                        message: err.to_string(),
                    });
                    if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        break format!("{MAX_CONSECUTIVE_FAILURES} consecutive tick failures");
                    }
                }
            }
            if let Some(reason) = self.risk.halted() {
                break format!("risk halt: {reason}");
            }
            if shutdown.load(Ordering::Relaxed) {
                break "interrupted".to_string();
            }
            // Sleep in short slices so Ctrl-C is felt immediately.
            let elapsed = started.elapsed();
            let mut remaining = interval.saturating_sub(elapsed);
            while remaining > Duration::ZERO && !shutdown.load(Ordering::Relaxed) {
                let slice = remaining.min(Duration::from_millis(200));
                std::thread::sleep(slice);
                remaining -= slice;
            }
        };

        self.journal.record(&Event::Stop {
            ts: now(),
            reason: stop_reason.clone(),
            realized_pnl: self.portfolio.realized_pnl,
            trades: self.portfolio.trade_count,
        });
        tracing::info!(reason = %stop_reason, "stopped");
        Ok(())
    }

    /// Run exactly one tick.
    pub fn tick(&mut self) -> Result<TickSummary> {
        let ts = now();
        let pools: Vec<_> = self.markets.iter().map(|m| m.pool).collect();
        let states = self.fetcher.fetch_pools(&pools)?;
        let current_point = self.fetcher.current_point()?;
        self.consecutive_failures = 0;

        if self.sync_balances {
            match self.balances.balances(&self.markets) {
                Ok((gp, items)) => self.portfolio.sync_balances(gp, &items),
                Err(err) => {
                    tracing::error!(%err, "balance sync failed; trading on the last known book");
                    self.journal.record(&Event::Error {
                        ts,
                        context: "balances".to_string(),
                        message: err.to_string(),
                    });
                }
            }
        }
        let sol_balance = match self.balances.sol_balance() {
            Ok(sol) => sol,
            Err(err) => {
                tracing::warn!(%err, "SOL balance unavailable");
                None
            }
        };

        let mut summary = TickSummary::default();
        let mut prices: BTreeMap<String, f64> = BTreeMap::new();

        for (market, state) in self.markets.iter().zip(states.iter()) {
            let Some(pool) = state else {
                summary.markets_unavailable += 1;
                continue;
            };
            summary.markets_priced += 1;
            let price = pool.spot_price();
            prices.insert(market.key.clone(), price);
            self.histories.record(&market.key, ts, price);

            if self.record_snapshots {
                let view = build_view(
                    market,
                    pool,
                    self.histories.get(&market.key),
                    &self.portfolio,
                    ts,
                    current_point,
                );
                self.journal.record(&Event::Snapshot {
                    ts,
                    market: market.key.clone(),
                    price,
                    floor: view.floor(),
                    exit_depth_gp: view.exit_depth_gp(),
                });
            }

            // Strategies are consulted one at a time so each sees the position as
            // it stands after any earlier strategy has already traded this tick.
            for index in 0..self.strategies.len() {
                let signals = {
                    let view = build_view(
                        market,
                        pool,
                        self.histories.get(&market.key),
                        &self.portfolio,
                        ts,
                        current_point,
                    );
                    self.strategies[index].on_market(&view)
                };
                summary.signals += signals.len();

                for signal in signals {
                    let name = self.strategies[index].name().to_string();
                    let view = build_view(
                        market,
                        pool,
                        self.histories.get(&market.key),
                        &self.portfolio,
                        ts,
                        current_point,
                    );
                    match self
                        .risk
                        .evaluate(&name, &signal, &view, &self.portfolio, sol_balance)
                    {
                        Decision::Reject(reason) => {
                            summary.rejected += 1;
                            tracing::debug!(%reason, "rejected");
                            self.journal.record(&Event::Reject { ts, reason });
                        }
                        Decision::Approve(order) => {
                            summary.approved += 1;
                            match self.executor.execute(&order, &view) {
                                Ok(fill) => {
                                    summary.fills += 1;
                                    tracing::info!(
                                        market = %fill.market,
                                        side = fill.side.as_str(),
                                        items = fill.items,
                                        gp = format_args!("{:.2}", fill.gp),
                                        price = format_args!("{:.2}", fill.price),
                                        strategy = %fill.strategy,
                                        "fill"
                                    );
                                    self.portfolio.apply_fill(&fill);
                                    self.risk.record_fill(&fill);
                                    self.strategies[index].on_fill(&fill);
                                    self.journal.record(&Event::Fill(Box::new(fill)));
                                }
                                Err(err) => {
                                    summary.errors += 1;
                                    tracing::warn!(market = %order.market, %err, "execution failed");
                                    self.journal.record(&Event::Error {
                                        ts,
                                        context: format!(
                                            "execute {} {}",
                                            order.side.as_str(),
                                            order.market
                                        ),
                                        message: err.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        self.journal.record(&Event::Tick {
            ts,
            markets: summary.markets_priced,
            signals: summary.signals,
            fills: summary.fills,
            gp: self.portfolio.gp,
            equity: self.portfolio.equity(&prices),
            realized_pnl: self.portfolio.realized_pnl,
        });
        Ok(summary)
    }
}

fn build_view<'a>(
    market: &'a Market,
    pool: &'a PoolState,
    history: &'a crate::market::PriceHistory,
    portfolio: &Portfolio,
    now: i64,
    current_point: u64,
) -> MarketView<'a> {
    MarketView {
        market,
        pool,
        history,
        position: portfolio.position(&market.key),
        now,
        current_point,
        gp_available: portfolio.gp,
    }
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RiskConfig;
    use crate::execution::PaperExecutor;
    use crate::strategy::{AlchFloorParams, AlchFloorStrategy};
    use crate::testing::{synthetic_market, synthetic_pool};
    use solana_sdk::pubkey::Pubkey;

    /// A fetcher that replays fixed pool states, so a tick is fully deterministic.
    struct StaticPools {
        states: Vec<Option<PoolState>>,
        point: u64,
    }

    impl PoolFetch for StaticPools {
        fn fetch_pools(&self, _pools: &[Pubkey]) -> mercantile_dex::Result<Vec<Option<PoolState>>> {
            Ok(self.states.clone())
        }
        fn current_point(&self) -> mercantile_dex::Result<u64> {
            Ok(self.point)
        }
    }

    fn engine_with(premium: f64, gp: f64) -> Engine<StaticPools, PaperExecutor, NoBalances> {
        let pool = synthetic_pool(54.0, premium, 100);
        let market = synthetic_market("lobster", 54.0, pool.token_a_mint);
        Engine::new(
            vec![market],
            StaticPools {
                states: vec![Some(pool)],
                point: now() as u64,
            },
            PaperExecutor::new(0.0),
            NoBalances,
            vec![Box::new(AlchFloorStrategy::new(AlchFloorParams::default()))],
            RiskManager::new(RiskConfig {
                trade_cooldown_secs: 0,
                ..Default::default()
            }),
            Portfolio::new(gp),
            64,
            Journal::none(),
        )
    }

    #[test]
    fn a_tick_at_the_floor_buys_and_books_the_position() {
        let mut engine = engine_with(1.0, 100_000.0);
        let summary = engine.tick().unwrap();
        assert_eq!(summary.markets_priced, 1);
        assert_eq!(summary.fills, 1);
        assert_eq!(summary.errors, 0);
        let position = engine.portfolio().position("lobster");
        assert!(position.items > 0);
        assert!(engine.portfolio().gp < 100_000.0);
    }

    #[test]
    fn a_tick_well_above_the_floor_does_nothing() {
        let mut engine = engine_with(1.5, 100_000.0);
        let summary = engine.tick().unwrap();
        assert_eq!(summary.signals, 0);
        assert_eq!(summary.fills, 0);
    }

    #[test]
    fn an_empty_wallet_produces_rejections_not_errors() {
        let mut engine = engine_with(1.0, 0.0);
        let summary = engine.tick().unwrap();
        assert_eq!(summary.fills, 0);
        assert_eq!(summary.rejected, 1);
        assert_eq!(summary.errors, 0);
    }

    #[test]
    fn unavailable_pools_are_counted_and_skipped() {
        let market = synthetic_market("lobster", 54.0, Pubkey::new_unique());
        let mut engine = Engine::new(
            vec![market],
            StaticPools {
                states: vec![None],
                point: now() as u64,
            },
            PaperExecutor::new(0.0),
            NoBalances,
            vec![Box::new(AlchFloorStrategy::new(AlchFloorParams::default()))],
            RiskManager::new(RiskConfig::default()),
            Portfolio::new(1_000.0),
            64,
            Journal::none(),
        );
        let summary = engine.tick().unwrap();
        assert_eq!(summary.markets_priced, 0);
        assert_eq!(summary.markets_unavailable, 1);
        assert_eq!(summary.fills, 0);
    }

    #[test]
    fn repeated_ticks_respect_the_position_cap() {
        let mut engine = engine_with(1.0, 1_000_000.0);
        for _ in 0..40 {
            engine.tick().unwrap();
        }
        let position = engine.portfolio().position("lobster");
        assert!(
            position.items <= 50,
            "alch-floor max_items is 50, got {}",
            position.items
        );
    }
}

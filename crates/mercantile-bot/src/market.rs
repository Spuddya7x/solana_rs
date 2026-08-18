//! Market snapshots and the price history strategies reason over.

use std::collections::{HashMap, VecDeque};

use mercantile_core::Market;
use mercantile_dex::pool::PoolState;
use mercantile_dex::quote::{Quote, Side, SwapQuote};
use solana_sdk::pubkey::Pubkey;

use crate::portfolio::Position;

/// A rolling window of prices for one market.
#[derive(Debug, Clone)]
pub struct PriceHistory {
    points: VecDeque<PricePoint>,
    capacity: usize,
}

/// One observation: GP per whole item at a moment.
#[derive(Debug, Clone, Copy)]
pub struct PricePoint {
    pub ts: i64,
    pub price: f64,
}

impl PriceHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            points: VecDeque::with_capacity(capacity.min(4_096)),
            capacity: capacity.max(2),
        }
    }

    /// Record a price, evicting the oldest point when full.
    pub fn push(&mut self, ts: i64, price: f64) {
        if self.points.len() == self.capacity {
            self.points.pop_front();
        }
        self.points.push_back(PricePoint { ts, price });
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn latest(&self) -> Option<PricePoint> {
        self.points.back().copied()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &PricePoint> {
        self.points.iter()
    }

    /// Mean price over the last `window` observations, if there are that many.
    pub fn mean(&self, window: usize) -> Option<f64> {
        if window == 0 || self.points.len() < window {
            return None;
        }
        let sum: f64 = self.points.iter().rev().take(window).map(|p| p.price).sum();
        Some(sum / window as f64)
    }

    /// Sample standard deviation over the last `window` observations.
    pub fn stdev(&self, window: usize) -> Option<f64> {
        if window < 2 {
            return None;
        }
        let mean = self.mean(window)?;
        let variance: f64 = self
            .points
            .iter()
            .rev()
            .take(window)
            .map(|p| (p.price - mean).powi(2))
            .sum::<f64>()
            / (window - 1) as f64;
        Some(variance.sqrt())
    }

    /// How many standard deviations the latest price sits from its rolling mean.
    ///
    /// `None` until the window is full, or when the window is flat (no dispersion
    /// means no signal, not an infinite one).
    pub fn zscore(&self, window: usize) -> Option<f64> {
        let latest = self.latest()?.price;
        let mean = self.mean(window)?;
        let stdev = self.stdev(window)?;
        if stdev <= f64::EPSILON {
            return None;
        }
        Some((latest - mean) / stdev)
    }
}

/// Everything a strategy sees about one market on one tick.
pub struct MarketView<'a> {
    pub market: &'a Market,
    pub pool: &'a PoolState,
    pub history: &'a PriceHistory,
    pub position: Position,
    /// Unix seconds for this tick.
    pub now: i64,
    /// The pool clock used for quoting (unix seconds on Mercantile pools).
    pub current_point: u64,
    /// GP the bot may still spend.
    pub gp_available: f64,
    /// Total on-chain supply of the item token, in whole items, when known.
    ///
    /// Scarcity is a claim about supply, so strategies that trade on it need
    /// this; it is a separate RPC read, so the engine refreshes it periodically
    /// rather than every tick.
    pub supply: Option<f64>,
}

impl<'a> MarketView<'a> {
    /// Registry key, the market's identity throughout the bot.
    pub fn key(&self) -> &str {
        &self.market.key
    }

    pub fn mint(&self) -> Pubkey {
        self.market.mint
    }

    pub fn pool_address(&self) -> Pubkey {
        self.market.pool
    }

    /// Current pool price in GP per whole item.
    pub fn spot(&self) -> f64 {
        self.pool.spot_price()
    }

    /// The permanent bid floor, `0.9 x lowalch` (or the pool's seed price for rares).
    pub fn floor(&self) -> f64 {
        self.pool.floor_price()
    }

    /// Spot as a multiple of the floor. 1.0 means the pool is sitting on its floor.
    pub fn premium(&self) -> f64 {
        self.pool.premium_over_floor()
    }

    /// GP the pool can still pay out before its price reaches the floor — the real
    /// depth of the "permanent bid", which is far smaller than the floor implies.
    pub fn exit_depth_gp(&self) -> f64 {
        self.pool
            .gp_reserve()
            .map(|base| mercantile_core::base_to_gp(base.min(u64::MAX as u128) as u64))
            .unwrap_or(0.0)
    }

    /// Whole items the pool could still sell.
    pub fn pool_items(&self) -> f64 {
        self.pool
            .item_reserve()
            .map(|base| mercantile_core::base_to_items(base.min(u64::MAX as u128) as u64))
            .unwrap_or(0.0)
    }

    /// Quote buying `items` whole items (exact-out).
    pub fn quote_buy(&self, items: u64) -> Option<Quote> {
        self.pool.quote_buy_items(items, self.current_point).ok()
    }

    /// Quote selling `items` whole items (exact-in).
    pub fn quote_sell(&self, items: u64) -> Option<Quote> {
        self.pool.quote_sell_items(items, self.current_point).ok()
    }

    /// The largest whole-item trade of `side` this pool can currently absorb,
    /// searched by halving from `max_items`. Strategies use it to size into thin pools.
    pub fn max_tradeable(&self, side: Side, max_items: u64) -> u64 {
        let mut size = max_items;
        while size > 0 {
            let ok = match side {
                Side::Buy => self.quote_buy(size).is_some(),
                Side::Sell => self.quote_sell(size).is_some(),
            };
            if ok {
                return size;
            }
            size /= 2;
        }
        0
    }
}

/// Per-market price history for the whole universe.
#[derive(Debug, Default)]
pub struct MarketHistories {
    histories: HashMap<String, PriceHistory>,
    capacity: usize,
}

impl MarketHistories {
    pub fn new(capacity: usize) -> Self {
        Self {
            histories: HashMap::new(),
            capacity,
        }
    }

    pub fn record(&mut self, key: &str, ts: i64, price: f64) {
        self.histories
            .entry(key.to_string())
            .or_insert_with(|| PriceHistory::new(self.capacity))
            .push(ts, price);
    }

    pub fn get(&mut self, key: &str) -> &PriceHistory {
        self.histories
            .entry(key.to_string())
            .or_insert_with(|| PriceHistory::new(self.capacity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_evicts_oldest_beyond_capacity() {
        let mut history = PriceHistory::new(3);
        for (i, price) in [10.0, 11.0, 12.0, 13.0].iter().enumerate() {
            history.push(i as i64, *price);
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.latest().unwrap().price, 13.0);
        assert_eq!(history.iter().next().unwrap().price, 11.0);
    }

    #[test]
    fn statistics_wait_for_a_full_window() {
        let mut history = PriceHistory::new(10);
        history.push(0, 10.0);
        assert!(history.mean(3).is_none());
        history.push(1, 12.0);
        history.push(2, 14.0);
        assert_eq!(history.mean(3).unwrap(), 12.0);
        assert!((history.stdev(3).unwrap() - 2.0).abs() < 1e-12);
        assert_eq!(history.zscore(3).unwrap(), 1.0);
    }

    #[test]
    fn a_flat_window_produces_no_signal() {
        let mut history = PriceHistory::new(10);
        for i in 0..5 {
            history.push(i, 42.0);
        }
        assert_eq!(history.stdev(5).unwrap(), 0.0);
        assert!(
            history.zscore(5).is_none(),
            "a flat market must not read as an infinite deviation"
        );
    }
}

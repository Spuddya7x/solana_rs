//! A ladder of buys and sells anchored to the alch floor.
//!
//! The floor gives every Mercantile market a natural, absolute anchor that does
//! not drift — unlike a moving average, `0.9 x lowalch` is fixed by the game's own
//! item values. Rungs are placed as fixed percentage steps above it, and the
//! strategy trades each crossing: buy a rung down, sell a rung up.
//!
//! State is one rung index per market, so a restart re-anchors on the first tick
//! rather than firing a burst of catch-up trades.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{Signal, Strategy};
use crate::market::MarketView;
use mercantile_dex::quote::Side;

pub const NAME: &str = "grid";

/// Parameters for [`GridStrategy`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GridParams {
    /// Rung spacing as a percentage of the floor price.
    pub step_pct: f64,
    /// How many rungs above the floor the ladder covers. Above the top rung the
    /// strategy only sells.
    pub levels: usize,
    /// Whole items per rung.
    pub size_items: u64,
    /// Stop adding once the position reaches this many items.
    pub max_items: u64,
    /// Refuse to sell below this multiple of the position's average cost, so a
    /// ladder walking down does not book losses rung by rung.
    pub min_sell_cost_mult: f64,
    /// Staleness tolerance around the quoted execution price, as a percentage.
    pub limit_tolerance_pct: f64,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            step_pct: 10.0,
            levels: 5,
            size_items: 2,
            max_items: 20,
            min_sell_cost_mult: 1.02,
            limit_tolerance_pct: 0.5,
        }
    }
}

impl GridParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.step_pct <= 0.0 {
            anyhow::bail!("grid.step_pct must be positive");
        }
        if self.levels == 0 {
            anyhow::bail!("grid.levels must be at least 1");
        }
        if self.size_items == 0 {
            anyhow::bail!("grid.size_items must be at least 1");
        }
        if self.min_sell_cost_mult < 1.0 {
            anyhow::bail!("grid.min_sell_cost_mult below 1.0 would sell at a loss by design");
        }
        Ok(())
    }

    /// Which rung a given floor premium sits on. Rung 0 is the floor itself.
    fn rung_for(&self, premium: f64) -> usize {
        let above_floor = (premium - 1.0).max(0.0) * 100.0;
        ((above_floor / self.step_pct).floor() as usize).min(self.levels)
    }
}

/// See the module docs.
pub struct GridStrategy {
    params: GridParams,
    /// Last rung observed per market.
    rungs: HashMap<String, usize>,
}

impl GridStrategy {
    pub fn new(params: GridParams) -> Self {
        Self {
            params,
            rungs: HashMap::new(),
        }
    }

    /// The rung the strategy last saw for a market, if any.
    pub fn rung(&self, key: &str) -> Option<usize> {
        self.rungs.get(key).copied()
    }
}

impl Strategy for GridStrategy {
    fn name(&self) -> &str {
        NAME
    }

    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal> {
        let premium = view.premium();
        if !premium.is_finite() {
            return Vec::new();
        }
        let rung = self.params.rung_for(premium);
        let Some(previous) = self.rungs.insert(view.key().to_string(), rung) else {
            // First sight of this market: anchor, do not trade.
            return Vec::new();
        };
        if rung == previous {
            return Vec::new();
        }

        let position = view.position;
        let spot = view.spot();

        if rung > previous {
            if position.items == 0 {
                return Vec::new();
            }
            let floor_limit = position.avg_cost_gp * self.params.min_sell_cost_mult;
            if position.avg_cost_gp > 0.0 && spot < floor_limit {
                return Vec::new();
            }
            let steps = (rung - previous) as u64;
            let wanted = (self.params.size_items * steps).min(position.items);
            let size = view.max_tradeable(Side::Sell, wanted);
            let Some(quote) = (size > 0).then(|| view.quote_sell(size)).flatten() else {
                return Vec::new();
            };
            // The ladder is a reason to sell; selling under cost is not.
            if position.avg_cost_gp > 0.0 && quote.execution_price < floor_limit {
                return Vec::new();
            }
            let limit = super::execution_limit(
                Side::Sell,
                quote.execution_price,
                self.params.limit_tolerance_pct,
            );
            return vec![Signal::sell(
                size,
                format!(
                    "crossed up to rung {rung} of {} at {spot:.2}",
                    self.params.levels
                ),
            )
            .with_limit(limit)];
        }

        // Crossed down a rung: add, unless the ladder is already full.
        if position.items >= self.params.max_items {
            return Vec::new();
        }
        let steps = (previous - rung) as u64;
        let room = self.params.max_items.saturating_sub(position.items);
        let wanted = (self.params.size_items * steps).min(room);
        let size = view.max_tradeable(Side::Buy, wanted);
        let Some(quote) = (size > 0).then(|| view.quote_buy(size)).flatten() else {
            return Vec::new();
        };
        let limit = super::execution_limit(
            Side::Buy,
            quote.execution_price,
            self.params.limit_tolerance_pct,
        );
        vec![Signal::buy(
            size,
            format!(
                "crossed down to rung {rung} of {} at {spot:.2}",
                self.params.levels
            ),
        )
        .with_limit(limit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestMarket;

    #[test]
    fn the_first_tick_only_anchors() {
        let mut strategy = GridStrategy::new(GridParams::default());
        let market = TestMarket::above_floor(1.25);
        assert!(strategy.on_market(&market.view()).is_empty());
        assert_eq!(strategy.rung("lobster"), Some(2));
    }

    #[test]
    fn buys_on_the_way_down_and_sells_on_the_way_up() {
        let mut strategy = GridStrategy::new(GridParams::default());
        strategy.on_market(&TestMarket::above_floor(1.25).view()); // anchor at rung 2

        let down = TestMarket::above_floor(1.05);
        let signals = strategy.on_market(&down.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Buy);
        assert_eq!(signals[0].items, 4, "two rungs crossed, two items each");

        let up = TestMarket::above_floor(1.35).with_position(6, 55.0);
        let signals = strategy.on_market(&up.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Sell);
    }

    #[test]
    fn holding_a_rung_produces_nothing() {
        let mut strategy = GridStrategy::new(GridParams::default());
        strategy.on_market(&TestMarket::above_floor(1.25).view());
        assert!(strategy
            .on_market(&TestMarket::above_floor(1.28).view())
            .is_empty());
    }

    #[test]
    fn refuses_to_ladder_out_below_cost() {
        let mut strategy = GridStrategy::new(GridParams::default());
        strategy.on_market(&TestMarket::above_floor(1.05).view());
        // Price rose a rung, but the position was bought far higher.
        let up = TestMarket::above_floor(1.15).with_position(6, 200.0);
        assert!(
            strategy.on_market(&up.view()).is_empty(),
            "a rising rung is not a reason to book a loss"
        );
    }

    #[test]
    fn rungs_are_clamped_to_the_configured_levels() {
        let params = GridParams {
            step_pct: 10.0,
            levels: 3,
            ..Default::default()
        };
        assert_eq!(params.rung_for(1.0), 0);
        assert_eq!(params.rung_for(1.05), 0);
        assert_eq!(params.rung_for(1.10), 1);
        assert_eq!(params.rung_for(1.99), 3, "clamped at levels");
        assert_eq!(params.rung_for(0.5), 0, "below the floor is still rung 0");
    }

    #[test]
    fn loss_making_parameters_are_rejected() {
        assert!(GridParams {
            min_sell_cost_mult: 0.9,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(GridParams {
            step_pct: 0.0,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}

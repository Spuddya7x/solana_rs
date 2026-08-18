//! Buy only what high alchemy would pay for.
//!
//! Unlike the other strategies, this one has no view on where the price is going.
//! It compares the live cost of buying a stack against a fixed, game-defined
//! payout — `0.6 x cost` per item, less a nature rune — and buys when the gap is
//! wide enough. The exit is not the pool: it is the Exchange Clerk, the alch
//! spell, and a withdrawal of the resulting GP.
//!
//! That makes it the one strategy here whose profit does not depend on anyone
//! else showing up to trade. It also makes it the one with an off-chain
//! obligation: the items it buys are only worth what it paid if something
//! actually casts the spell on them.

use serde::{Deserialize, Serialize};

use super::{Signal, Strategy};
use crate::alch::{best_size, RuneCost};
use crate::market::MarketView;
use mercantile_dex::quote::Side;

pub const NAME: &str = "alch-arb";

/// Parameters for [`AlchArbStrategy`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AlchArbParams {
    /// Minimum profit, as a fraction of the GP spent, before buying.
    pub min_margin: f64,
    /// Minimum absolute profit in GP for a stack, so tiny edges are left alone.
    pub min_profit_gp: f64,
    /// Largest stack to consider in one trade.
    pub max_items: u64,
    /// Stop buying a market once this many items are held and unalched.
    pub max_position_items: u64,
    /// GP per nature rune. Left at zero, the strategy reads it from the nature
    /// rune market if that market is in the universe, and otherwise assumes the
    /// rune's own pool floor of 7.2 GP.
    pub nature_rune_gp: f64,
    /// GP per fire rune. Zero assumes a staff of fire, which is how this should
    /// be run — the staff removes five runes per cast permanently.
    pub fire_rune_gp: f64,
    /// Staleness tolerance around the quoted execution price, as a percentage.
    pub limit_tolerance_pct: f64,
}

impl Default for AlchArbParams {
    fn default() -> Self {
        Self {
            min_margin: 0.25,
            min_profit_gp: 100.0,
            max_items: 25,
            max_position_items: 100,
            nature_rune_gp: 0.0,
            fire_rune_gp: 0.0,
            limit_tolerance_pct: 0.5,
        }
    }
}

impl AlchArbParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.min_margin <= 0.0 {
            anyhow::bail!("alch-arb.min_margin must be positive");
        }
        // 5/3 is the gross ratio at the floor; a margin at or above 2/3 is
        // unreachable once fees, impact and runes are paid.
        if self.min_margin >= mercantile_core::alch::floor_to_high_alch_ratio() - 1.0 {
            anyhow::bail!(
                "alch-arb.min_margin of {:.2} exceeds the {:.2} gross margin available at the floor",
                self.min_margin,
                mercantile_core::alch::floor_to_high_alch_ratio() - 1.0
            );
        }
        if self.max_items == 0 {
            anyhow::bail!("alch-arb.max_items must be at least 1");
        }
        if self.nature_rune_gp < 0.0 || self.fire_rune_gp < 0.0 {
            anyhow::bail!("alch-arb rune prices cannot be negative");
        }
        Ok(())
    }

    fn runes(&self, observed_nature_rune_gp: Option<f64>) -> RuneCost {
        // The nature rune's own pool floor: 0.9 x lowalch(20) = 7.2 GP.
        const NATURE_RUNE_FLOOR_GP: f64 = 7.2;
        let nature_rune_gp = if self.nature_rune_gp > 0.0 {
            self.nature_rune_gp
        } else {
            observed_nature_rune_gp.unwrap_or(NATURE_RUNE_FLOOR_GP)
        };
        RuneCost {
            nature_rune_gp,
            fire_rune_gp: self.fire_rune_gp,
        }
    }
}

/// Registry key of the nature rune market.
pub const NATURE_RUNE_KEY: &str = "naturerune";

/// See the module docs.
pub struct AlchArbStrategy {
    params: AlchArbParams,
    /// Last observed nature rune price, if that market is being watched.
    nature_rune_gp: Option<f64>,
}

impl AlchArbStrategy {
    pub fn new(params: AlchArbParams) -> Self {
        Self {
            params,
            nature_rune_gp: None,
        }
    }

    /// The rune price the strategy is currently costing casts at.
    pub fn rune_cost(&self) -> RuneCost {
        self.params.runes(self.nature_rune_gp)
    }
}

impl Strategy for AlchArbStrategy {
    fn name(&self) -> &str {
        NAME
    }

    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal> {
        // Watching the rune market is how casts get priced; it is never itself a
        // target, since 0.24 x 20 GP cannot pay for the rune it consumes.
        if view.key() == NATURE_RUNE_KEY {
            self.nature_rune_gp = Some(view.spot());
            return Vec::new();
        }
        if view.position.items >= self.params.max_position_items {
            return Vec::new();
        }

        let room = self
            .params
            .max_position_items
            .saturating_sub(view.position.items)
            .min(self.params.max_items);
        if room == 0 {
            return Vec::new();
        }

        let runes = self.params.runes(self.nature_rune_gp);
        let opportunity = best_size(view.market, view.pool, room, runes, view.current_point);
        let Some(opportunity) = opportunity else {
            return Vec::new();
        };
        if opportunity.margin < self.params.min_margin
            || opportunity.profit_gp < self.params.min_profit_gp
        {
            return Vec::new();
        }

        let limit = super::execution_limit(
            Side::Buy,
            opportunity.avg_price,
            self.params.limit_tolerance_pct,
        );
        vec![Signal::buy(
            opportunity.items,
            format!(
                "alch arb: {} items at {:.2} GP each alch for {} GP, {:+.0} GP ({:.0}%) after {:.1} GP of runes",
                opportunity.items,
                opportunity.avg_price,
                opportunity.high_alch_gp,
                opportunity.profit_gp,
                opportunity.margin * 100.0,
                opportunity.rune_gp,
            ),
        )
        .with_limit(limit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestMarket;

    /// A market whose floor is the usual 0.36 x cost for the given shop cost.
    fn market_for(cost: u64, premium: f64) -> TestMarket {
        let lowalch = mercantile_core::low_alch_value(cost);
        let mut market = TestMarket::new("item", lowalch as f64 * 0.9, premium);
        market.market.item.cost = cost;
        market.market.item.lowalch = lowalch;
        market
    }

    #[test]
    fn buys_a_stack_that_alchs_profitably() {
        let mut strategy = AlchArbStrategy::new(AlchArbParams::default());
        let market = market_for(5_000, 1.0);
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Buy);
        assert!(signals[0].reason.contains("alch"));
    }

    #[test]
    fn ignores_items_too_cheap_to_carry_a_nature_rune() {
        let mut strategy = AlchArbStrategy::new(AlchArbParams::default());
        let market = market_for(20, 1.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn ignores_a_pool_that_has_already_run_up() {
        let mut strategy = AlchArbStrategy::new(AlchArbParams::default());
        let market = market_for(5_000, 1.6);
        assert!(
            strategy.on_market(&market.view()).is_empty(),
            "past 5/3 of the floor the alch no longer pays"
        );
    }

    #[test]
    fn prices_casts_from_the_live_rune_market_when_it_is_watched() {
        let mut strategy = AlchArbStrategy::new(AlchArbParams::default());
        assert_eq!(
            strategy.rune_cost().nature_rune_gp,
            7.2,
            "falls back to the rune floor"
        );

        let mut runes = TestMarket::new(NATURE_RUNE_KEY, 7.2, 3.0);
        runes.market.key = NATURE_RUNE_KEY.to_string();
        let signals = strategy.on_market(&runes.view());
        assert!(
            signals.is_empty(),
            "the rune market is an input, never a target"
        );
        assert!((strategy.rune_cost().nature_rune_gp - 21.6).abs() < 1e-3);
    }

    #[test]
    fn expensive_runes_shrink_the_opportunity() {
        let cheap = {
            let mut strategy = AlchArbStrategy::new(AlchArbParams {
                nature_rune_gp: 7.2,
                min_profit_gp: 0.0,
                ..Default::default()
            });
            strategy.on_market(&market_for(300, 1.0).view())
        };
        let dear = {
            let mut strategy = AlchArbStrategy::new(AlchArbParams {
                nature_rune_gp: 60.0,
                min_profit_gp: 0.0,
                ..Default::default()
            });
            strategy.on_market(&market_for(300, 1.0).view())
        };
        assert!(!cheap.is_empty());
        assert!(
            dear.is_empty(),
            "a 60 GP rune eats the margin on a 300 GP item"
        );
    }

    #[test]
    fn stops_buying_once_the_unalched_pile_is_big_enough() {
        let mut strategy = AlchArbStrategy::new(AlchArbParams {
            max_position_items: 10,
            ..Default::default()
        });
        let market = market_for(5_000, 1.0).with_position(10, 1.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn unreachable_margins_are_rejected_at_config_time() {
        assert!(AlchArbParams {
            min_margin: 1.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(AlchArbParams {
            min_margin: 0.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(AlchArbParams::default().validate().is_ok());
    }
}

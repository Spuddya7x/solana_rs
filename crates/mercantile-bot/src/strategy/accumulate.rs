//! Convert GP into things that cannot be printed.
//!
//! The end of the pipeline. Everything upstream — training an account, alching
//! items, bridging the proceeds out — produces GP, and GP is the one asset in
//! this economy whose supply is unbounded: 10 billion at genesis, plus whatever
//! alchemy mints. Holding the output of the machine is holding the thing the
//! machine dilutes.
//!
//! So this strategy spends GP down to a floor on the scarcest items available,
//! preferring provable scarcity over price. It never sells: this is the exit,
//! not a trade.

use serde::{Deserialize, Serialize};

use super::{Signal, Strategy};
use crate::market::MarketView;
use crate::scarcity::{assess, Scarcity};
use mercantile_dex::quote::Side;

pub const NAME: &str = "accumulate";

/// Parameters for [`AccumulateStrategy`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccumulateParams {
    /// Only buy items scoring at least this on [`Scarcity`]. The default admits
    /// items with no in-game source and little bridged-in supply, and excludes
    /// everything the game can still produce.
    pub min_score: f64,
    /// Keep this much GP unspent — working capital for the alch loop, which is
    /// what generates the GP in the first place.
    pub reserve_gp: f64,
    /// Whole items per buy.
    pub size_items: u64,
    /// Stop once this many are held in one market.
    pub max_items: u64,
    /// Ignore markets whose floor is below this many GP.
    ///
    /// Scarcity is necessary but not sufficient: most items with no in-game
    /// source are macro-event cubes and quest litter floored at 0.9 GP. The
    /// floor is the protocol's own valuation, so this is how the strategy
    /// separates a party hat from a cooking pot.
    pub min_floor_gp: f64,
    /// Refuse to pay more than this multiple of the pool floor. Scarce does not
    /// mean worth any price, and these pools are thin enough to run away.
    pub max_premium: f64,
    /// Staleness tolerance around the quoted execution price, as a percentage.
    pub limit_tolerance_pct: f64,
}

impl Default for AccumulateParams {
    fn default() -> Self {
        Self {
            min_score: 0.5,
            min_floor_gp: 10_000.0,
            reserve_gp: 100_000.0,
            size_items: 1,
            max_items: 10,
            max_premium: 1.5,
            limit_tolerance_pct: 0.5,
        }
    }
}

impl AccumulateParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(0.0..=1.0).contains(&self.min_score) {
            anyhow::bail!("accumulate.min_score must be between 0 and 1");
        }
        if self.size_items == 0 {
            anyhow::bail!("accumulate.size_items must be at least 1");
        }
        if self.max_premium < 1.0 {
            anyhow::bail!("accumulate.max_premium below 1.0 would never fill — nothing trades below its floor");
        }
        if self.reserve_gp < 0.0 {
            anyhow::bail!("accumulate.reserve_gp cannot be negative");
        }
        Ok(())
    }
}

/// See the module docs.
pub struct AccumulateStrategy {
    params: AccumulateParams,
}

impl AccumulateStrategy {
    pub fn new(params: AccumulateParams) -> Self {
        Self { params }
    }

    /// How this strategy sees a market right now.
    pub fn assess(&self, view: &MarketView<'_>) -> Option<Scarcity> {
        // Without a supply reading there is no evidence, and this strategy does
        // not buy on faith.
        let supply = view.supply?;
        Some(assess(
            view.market,
            view.pool,
            supply,
            self.params.size_items,
            view.current_point,
        ))
    }
}

impl Strategy for AccumulateStrategy {
    fn name(&self) -> &str {
        NAME
    }

    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal> {
        if view.position.items >= self.params.max_items {
            return Vec::new();
        }
        if view.gp_available <= self.params.reserve_gp {
            return Vec::new();
        }
        let Some(scarcity) = self.assess(view) else {
            return Vec::new();
        };
        if scarcity.score < self.params.min_score {
            return Vec::new();
        }
        if scarcity.floor < self.params.min_floor_gp {
            return Vec::new();
        }
        if view.premium() > self.params.max_premium {
            return Vec::new();
        }

        let room = self
            .params
            .max_items
            .saturating_sub(view.position.items)
            .min(self.params.size_items);
        let size = view.max_tradeable(Side::Buy, room);
        if size == 0 {
            return Vec::new();
        }
        let Some(quote) = view.quote_buy(size) else {
            return Vec::new();
        };
        // Never dip into the reserve, whatever the opportunity.
        if view.gp_available - quote.gp() < self.params.reserve_gp {
            return Vec::new();
        }

        let limit = super::execution_limit(
            Side::Buy,
            quote.execution_price,
            self.params.limit_tolerance_pct,
        );
        vec![Signal::buy(
            size,
            format!(
                "scarcity {:.2}: {} supply {:.0}, {:.0} bridged in, {}",
                scarcity.score,
                scarcity.name,
                scarcity.supply,
                scarcity.bridged_in,
                scarcity.sources.describe(),
            ),
        )
        .with_limit(limit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestMarket;

    fn rare_market(key: &str, premium: f64) -> TestMarket {
        let mut market = TestMarket::new(key, 100_000.0, premium);
        market.market.key = key.to_string();
        market.with_gp(1_000_000.0)
    }

    #[test]
    fn buys_an_undiluted_rare() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams::default());
        let market = rare_market("red_partyhat", 1.0).with_supply(100.0);
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Buy);
        assert!(signals[0].reason.contains("no in-game source"));
    }

    #[test]
    fn ignores_scarce_but_worthless_litter() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams::default());
        // Capped, undiluted — and floored at 0.9 GP because nobody wants it.
        let mut market = TestMarket::new("macro_cube_bluestar", 0.9, 1.0);
        market.market.key = "macro_cube_bluestar".to_string();
        let market = market.with_gp(1_000_000.0).with_supply(100.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn ignores_anything_the_game_can_still_produce() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams::default());
        let market = rare_market("rune_platebody", 1.0).with_supply(100.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn will_not_buy_without_a_supply_reading() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams::default());
        let market = rare_market("red_partyhat", 1.0);
        assert!(
            strategy.on_market(&market.view()).is_empty(),
            "scarcity is a claim about supply; without supply there is no claim"
        );
    }

    #[test]
    fn a_diluted_rare_falls_below_the_threshold() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams::default());
        let market = rare_market("blue_partyhat", 1.0).with_supply(199.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn the_reserve_is_never_spent() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams {
            reserve_gp: 999_999_999.0,
            ..Default::default()
        });
        let market = rare_market("red_partyhat", 1.0).with_supply(100.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn a_pool_that_has_run_away_is_left_alone() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams::default());
        let market = rare_market("red_partyhat", 3.0).with_supply(100.0);
        assert!(
            strategy.on_market(&market.view()).is_empty(),
            "scarce is not the same as worth any price"
        );
    }

    #[test]
    fn stops_at_the_position_cap() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams {
            max_items: 2,
            ..Default::default()
        });
        let market = rare_market("red_partyhat", 1.0)
            .with_supply(100.0)
            .with_position(2, 1.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn it_never_sells() {
        let mut strategy = AccumulateStrategy::new(AccumulateParams::default());
        for premium in [1.0, 1.2, 1.4] {
            let market = rare_market("red_partyhat", premium)
                .with_supply(100.0)
                .with_position(1, 1.0);
            for signal in strategy.on_market(&market.view()) {
                assert_eq!(signal.side, Side::Buy, "this is the exit, not a trade");
            }
        }
    }

    #[test]
    fn impossible_parameters_are_rejected() {
        assert!(AccumulateParams {
            max_premium: 0.9,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(AccumulateParams {
            min_score: 2.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(AccumulateParams::default().validate().is_ok());
    }
}

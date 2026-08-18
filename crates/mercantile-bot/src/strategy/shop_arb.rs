//! Buy what an NPC shop will pay more for than the pool charges.
//!
//! The sibling of [`super::alch_arb`], and the one that works before level 55.
//! Alchemy pays a fixed `0.6 x cost`; a shopkeeper pays 60–95% of cost and needs
//! no spell, no runes and no Magic level — only a walk to the right counter.
//!
//! Like the alchemy strategy it has an obligation off chain: the items it buys
//! are only worth what it paid once something bridges them into the game and
//! sells them. `gamebot/moneymaker.ts` is that something.

use serde::{Deserialize, Serialize};

use super::{Signal, Strategy};
use crate::market::MarketView;
use crate::shopflip::best_size;
use mercantile_dex::quote::Side;

pub const NAME: &str = "shop-arb";

/// Parameters for [`ShopArbStrategy`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShopArbParams {
    /// Minimum profit as a fraction of GP spent.
    pub min_margin: f64,
    /// Minimum absolute profit in GP for a stack.
    pub min_profit_gp: f64,
    /// Largest stack to consider in one trade.
    pub max_items: u64,
    /// Stop buying a market once this many are held and unsold.
    pub max_position_items: u64,
    /// Staleness tolerance around the quoted execution price, as a percentage.
    pub limit_tolerance_pct: f64,
}

impl Default for ShopArbParams {
    fn default() -> Self {
        Self {
            min_margin: 0.2,
            min_profit_gp: 500.0,
            max_items: 25,
            max_position_items: 100,
            limit_tolerance_pct: 0.5,
        }
    }
}

impl ShopArbParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.min_margin <= 0.0 {
            anyhow::bail!("shop-arb.min_margin must be positive");
        }
        if self.max_items == 0 {
            anyhow::bail!("shop-arb.max_items must be at least 1");
        }
        Ok(())
    }
}

/// See the module docs.
pub struct ShopArbStrategy {
    params: ShopArbParams,
}

impl ShopArbStrategy {
    pub fn new(params: ShopArbParams) -> Self {
        Self { params }
    }
}

impl Strategy for ShopArbStrategy {
    fn name(&self) -> &str {
        NAME
    }

    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal> {
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

        let Some(flip) = best_size(view.market, view.pool, room, view.current_point) else {
            return Vec::new();
        };
        if flip.margin < self.params.min_margin || flip.profit_gp < self.params.min_profit_gp {
            return Vec::new();
        }

        let limit =
            super::execution_limit(Side::Buy, flip.avg_price, self.params.limit_tolerance_pct);
        vec![Signal::buy(
            flip.items,
            format!(
                "shop arb: {} at {:.0} GP each, {} pays {:.0} for the stack, {:+.0} GP ({:.0}%)",
                flip.items,
                flip.avg_price,
                flip.shop_title,
                flip.shop_revenue,
                flip.profit_gp,
                flip.margin * 100.0,
            ),
        )
        .with_limit(limit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestMarket;

    /// A market whose floor is the usual 0.36 x cost.
    fn market_for(key: &str, cost: u64, premium: f64) -> TestMarket {
        let lowalch = mercantile_core::low_alch_value(cost);
        let mut market = TestMarket::new(key, lowalch as f64 * 0.9, premium);
        market.market.key = key.to_string();
        market.market.item.cost = cost;
        market.market.item.lowalch = lowalch;
        market
    }

    #[test]
    fn buys_a_stack_a_shop_will_pay_for() {
        let mut strategy = ShopArbStrategy::new(ShopArbParams::default());
        let market = market_for("rune_platebody", 65_000, 1.0);
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Buy);
        assert!(signals[0].reason.contains("shop arb"));
    }

    #[test]
    fn needs_no_magic_level_unlike_the_alchemy_route() {
        // The point of this strategy: nothing about the signal depends on the
        // account's skills, only on the two prices.
        let mut strategy = ShopArbStrategy::new(ShopArbParams::default());
        assert!(!strategy
            .on_market(&market_for("rune_platebody", 65_000, 1.0).view())
            .is_empty());
    }

    #[test]
    fn ignores_worthless_items() {
        let mut strategy = ShopArbStrategy::new(ShopArbParams::default());
        assert!(strategy
            .on_market(&market_for("cow_hide", 1, 1.0).view())
            .is_empty());
    }

    #[test]
    fn ignores_a_pool_that_has_run_past_the_shop_bid() {
        let mut strategy = ShopArbStrategy::new(ShopArbParams::default());
        assert!(strategy
            .on_market(&market_for("rune_platebody", 65_000, 3.0).view())
            .is_empty());
    }

    #[test]
    fn stops_at_the_position_cap() {
        let mut strategy = ShopArbStrategy::new(ShopArbParams {
            max_position_items: 5,
            ..Default::default()
        });
        let market = market_for("rune_platebody", 65_000, 1.0).with_position(5, 1.0);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn impossible_parameters_are_rejected() {
        assert!(ShopArbParams {
            min_margin: 0.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ShopArbParams {
            max_items: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(ShopArbParams::default().validate().is_ok());
    }
}

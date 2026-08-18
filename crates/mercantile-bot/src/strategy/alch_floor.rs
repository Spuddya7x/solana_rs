//! Buy the permanent bid floor; sell into strength.
//!
//! This is the strategy the Mercantile market structure is built for. Every item
//! pool is seeded single-sided at `P0 = 0.9 x lowalch` over the range `[P0, inf)`,
//! so the price *cannot* trade below `P0`: an item bought at the floor has bounded
//! downside in GP terms, and the game's own alchemy value is the backstop.
//!
//! The catch, and the reason this strategy checks depth before it buys: the floor
//! is a price floor, not a size floor. The GP a pool can pay out is only the GP
//! previous buyers put in, so a pool can be sitting at its floor with almost no
//! bid depth behind it. Buying into that is how a "riskless" floor trade turns
//! into inventory you cannot sell.

use serde::{Deserialize, Serialize};

use super::{Signal, Strategy};
use crate::market::MarketView;

pub const NAME: &str = "alch-floor";

/// Parameters for [`AlchFloorStrategy`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AlchFloorParams {
    /// Buy while spot is at or below this multiple of the floor.
    pub buy_below_premium: f64,
    /// Sell once spot reaches this multiple of the floor.
    pub take_profit_premium: f64,
    /// Sell once spot reaches this multiple of the position's average cost, even
    /// if the floor premium target has not been hit.
    pub take_profit_cost_mult: f64,
    /// Whole items per buy.
    pub size_items: u64,
    /// Stop adding once the position reaches this many items.
    pub max_items: u64,
    /// How far past the quoted execution price a fill may still be accepted,
    /// as a percentage. This is a staleness guard, not an economic gate.
    pub limit_tolerance_pct: f64,
    /// Refuse to buy unless the pool could pay out this multiple of the trade's
    /// GP notional before hitting the floor.
    ///
    /// Defaults to `0.0` — off — because the pool is not the only exit: an item
    /// bought at `0.9 x lowalch` can be bridged back into the game and alched for
    /// `lowalch`, so the in-game economy backs the trade even when the pool's bid
    /// is thin. Set it above zero to insist on a pool-side exit instead, and
    /// expect far fewer fills: a pool sitting exactly on its floor holds no GP at
    /// all, and its bid depth only grows as other buyers push the price up.
    pub min_exit_depth_mult: f64,
}

impl Default for AlchFloorParams {
    fn default() -> Self {
        Self {
            buy_below_premium: 1.05,
            take_profit_premium: 1.30,
            take_profit_cost_mult: 1.15,
            size_items: 5,
            max_items: 50,
            limit_tolerance_pct: 0.5,
            min_exit_depth_mult: 0.0,
        }
    }
}

impl AlchFloorParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.buy_below_premium < 1.0 {
            anyhow::bail!(
                "alch-floor.buy_below_premium must be at least 1.0 — nothing ever trades below the floor"
            );
        }
        if self.take_profit_premium <= self.buy_below_premium {
            anyhow::bail!("alch-floor.take_profit_premium must exceed buy_below_premium");
        }
        if self.take_profit_cost_mult <= 1.0 {
            anyhow::bail!("alch-floor.take_profit_cost_mult must exceed 1.0");
        }
        if self.size_items == 0 {
            anyhow::bail!("alch-floor.size_items must be at least 1");
        }
        Ok(())
    }
}

/// See the module docs.
pub struct AlchFloorStrategy {
    params: AlchFloorParams,
}

impl AlchFloorStrategy {
    pub fn new(params: AlchFloorParams) -> Self {
        Self { params }
    }
}

impl Strategy for AlchFloorStrategy {
    fn name(&self) -> &str {
        NAME
    }

    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal> {
        let premium = view.premium();
        if !premium.is_finite() {
            return Vec::new();
        }
        let position = view.position;
        let spot = view.spot();
        let floor = view.floor();

        // Exit first: a position that has run is worth more than a new one.
        if position.items > 0 {
            let cost_target = position.avg_cost_gp * self.params.take_profit_cost_mult;
            let hit_floor_target = premium >= self.params.take_profit_premium;
            let hit_cost_target = position.avg_cost_gp > 0.0 && spot >= cost_target;
            if hit_floor_target || hit_cost_target {
                let size = view.max_tradeable(mercantile_dex::quote::Side::Sell, position.items);
                if let (true, Some(quote)) = (size > 0, view.quote_sell(size)) {
                    let reason = if hit_floor_target {
                        format!("spot {spot:.2} is {premium:.2}x the {floor:.2} floor")
                    } else {
                        format!(
                            "spot {spot:.2} is {:.2}x average cost {:.2}",
                            spot / position.avg_cost_gp,
                            position.avg_cost_gp
                        )
                    };
                    let limit = super::execution_limit(
                        mercantile_dex::quote::Side::Sell,
                        quote.execution_price,
                        self.params.limit_tolerance_pct,
                    );
                    return vec![Signal::sell(size, reason).with_limit(limit)];
                }
            }
            if position.items >= self.params.max_items {
                return Vec::new();
            }
        }

        if premium > self.params.buy_below_premium {
            return Vec::new();
        }

        let room = self.params.max_items.saturating_sub(position.items);
        let size = view.max_tradeable(
            mercantile_dex::quote::Side::Buy,
            self.params.size_items.min(room),
        );
        if size == 0 {
            return Vec::new();
        }

        // Only buy what the pool could plausibly buy back.
        let Some(quote) = view.quote_buy(size) else {
            return Vec::new();
        };
        let notional = quote.gp();
        let depth = view.exit_depth_gp();
        if self.params.min_exit_depth_mult > 0.0
            && depth < notional * self.params.min_exit_depth_mult
        {
            tracing::debug!(
                market = view.key(),
                depth,
                notional,
                "skipping floor buy: no exit depth"
            );
            return Vec::new();
        }

        let limit = super::execution_limit(
            mercantile_dex::quote::Side::Buy,
            quote.execution_price,
            self.params.limit_tolerance_pct,
        );
        vec![Signal::buy(
            size,
            format!("spot {spot:.2} within {premium:.3}x of the {floor:.2} floor, {depth:.0} GP of exit depth"),
        )
        .with_limit(limit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestMarket;
    use mercantile_dex::quote::Side;

    #[test]
    fn buys_when_the_pool_sits_on_its_floor() {
        let mut strategy = AlchFloorStrategy::new(AlchFloorParams::default());
        let market = TestMarket::at_floor();
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Buy);
        assert!(signals[0].items > 0);
        assert!(signals[0].reason.contains("floor"));
    }

    #[test]
    fn will_not_buy_a_pool_trading_well_above_its_floor() {
        let mut strategy = AlchFloorStrategy::new(AlchFloorParams::default());
        let market = TestMarket::above_floor(1.4);
        let signals = strategy.on_market(&market.view());
        assert!(signals.is_empty(), "{signals:?}");
    }

    #[test]
    fn takes_profit_once_the_premium_target_is_hit() {
        let mut strategy = AlchFloorStrategy::new(AlchFloorParams::default());
        let market = TestMarket::above_floor(1.4).with_position(10, 50.0);
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Sell);
        assert!(signals[0].items > 0);
    }

    #[test]
    fn refuses_to_buy_a_floor_with_no_exit_depth() {
        let mut strategy = AlchFloorStrategy::new(AlchFloorParams {
            min_exit_depth_mult: 1_000.0,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.02);
        let signals = strategy.on_market(&market.view());
        assert!(
            signals.is_empty(),
            "a floor you cannot sell back into is not an opportunity"
        );
    }

    #[test]
    fn stops_adding_once_the_position_cap_is_reached() {
        let mut strategy = AlchFloorStrategy::new(AlchFloorParams {
            max_items: 10,
            ..Default::default()
        });
        let market = TestMarket::at_floor().with_position(10, 1.0);
        let signals = strategy.on_market(&market.view());
        assert!(signals.is_empty());
    }

    #[test]
    fn parameters_that_could_never_trade_are_rejected() {
        assert!(AlchFloorParams {
            buy_below_premium: 0.9,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(AlchFloorParams {
            take_profit_premium: 1.0,
            buy_below_premium: 1.05,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}

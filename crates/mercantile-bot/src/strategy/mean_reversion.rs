//! Fade moves away from a rolling mean.
//!
//! Mercantile pools are thin and driven by lumpy flow — a bot dumping a night's
//! grind, or a buyer clearing a shelf — so prices overshoot and come back. This
//! strategy buys when the price is unusually low against its own recent history
//! and sells when it has recovered.
//!
//! It refuses to act until it has a full window of observations, and it clamps
//! entries to a floor premium band: on this market "cheap relative to the last
//! hour" is only interesting while the price is still near the alch floor that
//! bounds it.

use serde::{Deserialize, Serialize};

use super::{Signal, Strategy};
use crate::market::MarketView;
use mercantile_dex::quote::Side;

pub const NAME: &str = "mean-reversion";

/// Parameters for [`MeanReversionStrategy`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MeanReversionParams {
    /// Observations in the rolling window.
    pub window: usize,
    /// Buy when the z-score falls to or below this (negative).
    pub entry_z: f64,
    /// Sell when the z-score rises to or above this.
    pub exit_z: f64,
    /// Whole items per entry.
    pub size_items: u64,
    /// Stop adding once the position reaches this many items.
    pub max_items: u64,
    /// Never enter above this multiple of the floor, however cheap it looks
    /// against its own history.
    pub max_entry_premium: f64,
    /// Exit regardless of z-score once the position is down this fraction from
    /// its average cost. `0.0` disables.
    pub stop_loss_pct: f64,
    /// Staleness tolerance around the quoted execution price, as a percentage.
    pub limit_tolerance_pct: f64,
}

impl Default for MeanReversionParams {
    fn default() -> Self {
        Self {
            window: 30,
            entry_z: -1.5,
            exit_z: 0.5,
            size_items: 3,
            max_items: 30,
            max_entry_premium: 2.0,
            stop_loss_pct: 0.0,
            limit_tolerance_pct: 0.5,
        }
    }
}

impl MeanReversionParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.window < 2 {
            anyhow::bail!("mean-reversion.window must be at least 2");
        }
        if self.entry_z >= 0.0 {
            anyhow::bail!("mean-reversion.entry_z must be negative — it is a dip threshold");
        }
        if self.exit_z <= self.entry_z {
            anyhow::bail!("mean-reversion.exit_z must be above entry_z");
        }
        if self.size_items == 0 {
            anyhow::bail!("mean-reversion.size_items must be at least 1");
        }
        if !(0.0..1.0).contains(&self.stop_loss_pct) {
            anyhow::bail!("mean-reversion.stop_loss_pct must be between 0 and 1");
        }
        Ok(())
    }
}

/// See the module docs.
pub struct MeanReversionStrategy {
    params: MeanReversionParams,
}

impl MeanReversionStrategy {
    pub fn new(params: MeanReversionParams) -> Self {
        Self { params }
    }
}

impl Strategy for MeanReversionStrategy {
    fn name(&self) -> &str {
        NAME
    }

    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal> {
        let Some(z) = view.history.zscore(self.params.window) else {
            // Not enough history yet, or a perfectly flat window: no signal either way.
            return Vec::new();
        };
        let position = view.position;
        let spot = view.spot();

        if position.items > 0 {
            let stopped = self.params.stop_loss_pct > 0.0
                && position.avg_cost_gp > 0.0
                && spot <= position.avg_cost_gp * (1.0 - self.params.stop_loss_pct);
            if z >= self.params.exit_z || stopped {
                let size = view.max_tradeable(Side::Sell, position.items);
                if let (true, Some(quote)) = (size > 0, view.quote_sell(size)) {
                    let reason = if stopped {
                        format!("stop loss: {spot:.2} vs cost {:.2}", position.avg_cost_gp)
                    } else {
                        format!("z {z:+.2} back above exit {:+.2}", self.params.exit_z)
                    };
                    let limit = super::execution_limit(
                        Side::Sell,
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

        if z > self.params.entry_z {
            return Vec::new();
        }
        if view.premium() > self.params.max_entry_premium {
            return Vec::new();
        }
        let room = self.params.max_items.saturating_sub(position.items);
        let size = view.max_tradeable(Side::Buy, self.params.size_items.min(room));
        if size == 0 {
            return Vec::new();
        }
        let mean = view.history.mean(self.params.window).unwrap_or(spot);
        let Some(quote) = view.quote_buy(size) else {
            return Vec::new();
        };
        // Paying more than the mean defeats the point of fading the dip.
        if quote.execution_price >= mean {
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
                "z {z:+.2} at or below entry {:+.2} (execution {:.2} vs mean {mean:.2})",
                self.params.entry_z, quote.execution_price
            ),
        )
        .with_limit(limit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestMarket;

    /// A window of prices ending well below its own mean.
    fn dipped_history() -> Vec<f64> {
        let mut prices = vec![70.0; 29];
        prices.push(60.0);
        prices
    }

    #[test]
    fn stays_quiet_until_the_window_is_full() {
        let mut strategy = MeanReversionStrategy::new(MeanReversionParams::default());
        let market = TestMarket::above_floor(1.1).with_history(&[70.0, 60.0]);
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn buys_a_dip_against_its_own_history() {
        let mut strategy = MeanReversionStrategy::new(MeanReversionParams::default());
        let market = TestMarket::above_floor(1.1).with_history(&dipped_history());
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Buy);
        assert!(signals[0].reason.contains('z'));
    }

    #[test]
    fn will_not_chase_a_dip_that_is_still_expensive_against_the_floor() {
        let mut strategy = MeanReversionStrategy::new(MeanReversionParams {
            max_entry_premium: 1.05,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.5).with_history(&dipped_history());
        assert!(strategy.on_market(&market.view()).is_empty());
    }

    #[test]
    fn sells_when_the_price_comes_back() {
        let mut strategy = MeanReversionStrategy::new(MeanReversionParams::default());
        let mut prices = vec![60.0; 29];
        prices.push(75.0);
        let market = TestMarket::above_floor(1.4)
            .with_history(&prices)
            .with_position(6, 60.0);
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Sell);
    }

    #[test]
    fn stop_loss_exits_even_while_the_dip_deepens() {
        let mut strategy = MeanReversionStrategy::new(MeanReversionParams {
            stop_loss_pct: 0.1,
            ..Default::default()
        });
        // Still a dip by z-score, so no exit signal would fire on its own.
        let market = TestMarket::above_floor(1.2)
            .with_history(&dipped_history())
            .with_position(6, 200.0);
        let signals = strategy.on_market(&market.view());
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].side, Side::Sell);
        assert!(signals[0].reason.contains("stop loss"));
    }

    #[test]
    fn nonsense_parameters_are_rejected() {
        assert!(MeanReversionParams {
            entry_z: 1.0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(MeanReversionParams {
            window: 1,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(MeanReversionParams {
            exit_z: -2.0,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}

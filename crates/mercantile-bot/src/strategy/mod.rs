//! The strategy interface and the built-in strategies.
//!
//! A strategy sees one market at a time through a [`MarketView`] — pool state,
//! price history, the current position, and live quoting — and returns [`Signal`]s.
//! It never sends anything: signals pass through the risk manager, which sizes and
//! vetoes, and only then reach an executor. That split is what lets the same
//! strategy code run in paper and live mode unchanged, and be unit-tested with no
//! network at all.

pub mod accumulate;
pub mod alch_arb;
pub mod alch_floor;
pub mod grid;
pub mod mean_reversion;
pub mod shop_arb;

use serde::{Deserialize, Serialize};

use crate::market::MarketView;
use mercantile_dex::quote::Side;

pub use accumulate::{AccumulateParams, AccumulateStrategy};
pub use alch_arb::{AlchArbParams, AlchArbStrategy};
pub use alch_floor::{AlchFloorParams, AlchFloorStrategy};
pub use grid::{GridParams, GridStrategy};
pub use mean_reversion::{MeanReversionParams, MeanReversionStrategy};
pub use shop_arb::{ShopArbParams, ShopArbStrategy};

/// Turn a quoted execution price into a limit price.
///
/// A strategy's limit exists to catch the pool *moving* between the tick that
/// decided to trade and the transaction that lands — not to re-litigate costs the
/// strategy already accepted. Mercantile pools are shallow enough that a five-item
/// buy routinely executes 7% above spot, so a limit derived from spot would veto
/// every trade the strategy just asked for. Deriving it from the quote instead
/// keeps the guard tight and the intent intact.
pub fn execution_limit(side: Side, execution_price: f64, tolerance_pct: f64) -> f64 {
    let tolerance = tolerance_pct.max(0.0) / 100.0;
    match side {
        Side::Buy => execution_price * (1.0 + tolerance),
        Side::Sell => execution_price * (1.0 - tolerance),
    }
}

/// A strategy's request to trade. Sizes are intentions, not commitments — the risk
/// manager may shrink or refuse them.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub side: Side,
    /// Whole items the strategy wants to trade.
    pub items: u64,
    /// Worst acceptable price in GP per item, if the strategy cares.
    pub limit_price: Option<f64>,
    /// Why, in a form worth reading in the journal.
    pub reason: String,
}

impl Signal {
    pub fn buy(items: u64, reason: impl Into<String>) -> Self {
        Self {
            side: Side::Buy,
            items,
            limit_price: None,
            reason: reason.into(),
        }
    }

    pub fn sell(items: u64, reason: impl Into<String>) -> Self {
        Self {
            side: Side::Sell,
            items,
            limit_price: None,
            reason: reason.into(),
        }
    }

    /// Attach a worst-acceptable price.
    pub fn with_limit(mut self, price: f64) -> Self {
        self.limit_price = Some(price);
        self
    }

    /// Whether `price` satisfies this signal's limit.
    pub fn accepts(&self, price: f64) -> bool {
        match (self.limit_price, self.side) {
            (None, _) => true,
            (Some(limit), Side::Buy) => price <= limit,
            (Some(limit), Side::Sell) => price >= limit,
        }
    }
}

/// A trading strategy.
pub trait Strategy: Send {
    /// Stable name, used in journals and logs.
    fn name(&self) -> &str;

    /// Decide what to do about one market this tick.
    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal>;

    /// Notified after one of this strategy's signals actually filled.
    fn on_fill(&mut self, _fill: &crate::execution::Fill) {}
}

/// One `[[strategies]]` entry from the config.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StrategyEntry {
    /// Whether to run it.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: StrategyKind,
}

fn default_true() -> bool {
    true
}

/// The built-in strategies and their parameters.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StrategyKind {
    /// Convert GP into items whose supply cannot grow.
    Accumulate(AccumulateParams),
    /// Buy only stacks that high alchemy would pay for.
    AlchArb(AlchArbParams),
    /// Buy stacks an NPC shop will pay more for than the pool charges.
    ShopArb(ShopArbParams),
    /// Buy at the permanent alch floor, sell into strength.
    AlchFloor(AlchFloorParams),
    /// Fade moves away from a rolling mean.
    MeanReversion(MeanReversionParams),
    /// Ladder buys and sells around the floor.
    Grid(GridParams),
}

impl StrategyEntry {
    /// Build the strategy this entry describes.
    pub fn build(&self) -> Box<dyn Strategy> {
        match &self.kind {
            StrategyKind::Accumulate(params) => Box::new(AccumulateStrategy::new(params.clone())),
            StrategyKind::AlchArb(params) => Box::new(AlchArbStrategy::new(params.clone())),
            StrategyKind::ShopArb(params) => Box::new(ShopArbStrategy::new(params.clone())),
            StrategyKind::AlchFloor(params) => Box::new(AlchFloorStrategy::new(params.clone())),
            StrategyKind::MeanReversion(params) => {
                Box::new(MeanReversionStrategy::new(params.clone()))
            }
            StrategyKind::Grid(params) => Box::new(GridStrategy::new(params.clone())),
        }
    }

    /// Reject parameter sets that could never trade, or could only lose.
    pub fn validate(&self) -> anyhow::Result<()> {
        match &self.kind {
            StrategyKind::Accumulate(params) => params.validate(),
            StrategyKind::AlchArb(params) => params.validate(),
            StrategyKind::ShopArb(params) => params.validate(),
            StrategyKind::AlchFloor(params) => params.validate(),
            StrategyKind::MeanReversion(params) => params.validate(),
            StrategyKind::Grid(params) => params.validate(),
        }
    }

    /// The name the built strategy will report.
    pub fn name(&self) -> &'static str {
        match self.kind {
            StrategyKind::Accumulate(_) => accumulate::NAME,
            StrategyKind::AlchArb(_) => alch_arb::NAME,
            StrategyKind::ShopArb(_) => shop_arb::NAME,
            StrategyKind::AlchFloor(_) => alch_floor::NAME,
            StrategyKind::MeanReversion(_) => mean_reversion::NAME,
            StrategyKind::Grid(_) => grid::NAME,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_limits_bracket_the_quote() {
        let buy = execution_limit(Side::Buy, 100.0, 0.5);
        let sell = execution_limit(Side::Sell, 100.0, 0.5);
        assert!((buy - 100.5).abs() < 1e-9);
        assert!((sell - 99.5).abs() < 1e-9);
        assert!(Signal::buy(1, "x").with_limit(buy).accepts(100.0));
        assert!(Signal::sell(1, "x").with_limit(sell).accepts(100.0));
    }

    #[test]
    fn limits_gate_the_right_direction() {
        let buy = Signal::buy(1, "test").with_limit(50.0);
        assert!(buy.accepts(49.0));
        assert!(!buy.accepts(51.0));

        let sell = Signal::sell(1, "test").with_limit(50.0);
        assert!(sell.accepts(51.0));
        assert!(!sell.accepts(49.0));

        assert!(
            Signal::buy(1, "test").accepts(f64::MAX),
            "no limit means no gate"
        );
    }

    #[test]
    fn config_entries_build_the_strategy_they_name() {
        let entries: Vec<StrategyEntry> = toml::from_str::<toml::Value>(
            r#"
            [[strategies]]
            kind = "alch-floor"
            buy_below_premium = 1.02

            [[strategies]]
            kind = "mean-reversion"
            enabled = false
            window = 20

            [[strategies]]
            kind = "grid"
            levels = 3
            "#,
        )
        .unwrap()["strategies"]
            .clone()
            .try_into()
            .unwrap();

        assert_eq!(entries.len(), 3);
        assert!(entries[0].enabled, "enabled defaults to true");
        assert!(!entries[1].enabled);
        assert_eq!(entries[0].name(), "alch-floor");
        assert_eq!(entries[1].name(), "mean-reversion");
        assert_eq!(entries[2].name(), "grid");
        assert_eq!(entries[0].build().name(), "alch-floor");
    }
}

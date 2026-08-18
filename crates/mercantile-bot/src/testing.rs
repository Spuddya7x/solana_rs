//! Synthetic markets for testing strategies and risk rules without a network.
//!
//! The pools built here are real [`PoolState`] values with the same shape as a
//! Mercantile pool — concentrated liquidity over `[P0, MAX)`, 1% fee collected in
//! GP — so quoting against them exercises the production math rather than a mock.

use mercantile_core::{sqrt_price_from_price, Item, Market};
use mercantile_dex::math::MAX_SQRT_PRICE;
use mercantile_dex::pool::{CollectFeeMode, DynamicFee, PoolFees, PoolState};
use solana_sdk::pubkey::Pubkey;

use crate::market::{MarketView, PriceHistory};
use crate::portfolio::Position;

/// A pool seeded like a Mercantile pool and then traded up to `premium` x floor.
///
/// `seeded_items` is roughly how many whole items the pool can still sell.
pub fn synthetic_pool(floor_gp: f64, premium: f64, seeded_items: u64) -> PoolState {
    let sqrt_min_price = sqrt_price_from_price(floor_gp);
    let sqrt_price = sqrt_price_from_price(floor_gp * premium);
    let item_base_units = seeded_items * 10u64.pow(mercantile_core::ITEM_DECIMALS as u32);
    // Above the floor the range runs to MAX_SQRT_PRICE, where
    // amount_a ~= liquidity / sqrt_price, so this seeds about `seeded_items`.
    let liquidity = item_base_units as u128 * sqrt_price;

    let mut base_fee_info = [0u8; 32];
    base_fee_info[0..8].copy_from_slice(&10_000_000u64.to_le_bytes()); // flat 1%

    PoolState {
        fees: PoolFees {
            base_fee_info,
            protocol_fee_percent: 20,
            referral_fee_percent: 20,
            compounding_fee_bps: 0,
            dynamic_fee: DynamicFee {
                initialized: false,
                max_volatility_accumulator: 0,
                variable_fee_control: 0,
                bin_step: 0,
                volatility_accumulator: 0,
            },
            init_sqrt_price: sqrt_min_price,
        },
        token_a_mint: Pubkey::new_unique(),
        token_b_mint: mercantile_core::GP_MINT,
        token_a_vault: Pubkey::new_unique(),
        token_b_vault: Pubkey::new_unique(),
        liquidity,
        sqrt_min_price,
        sqrt_max_price: MAX_SQRT_PRICE,
        sqrt_price,
        activation_point: 0,
        activation_type: 1,
        pool_status: 0,
        token_a_flag: 0,
        token_b_flag: 0,
        collect_fee_mode: CollectFeeMode::OnlyB,
        fee_version: 1,
        token_a_amount: item_base_units,
        token_b_amount: 0,
    }
}

/// A registry entry for a synthetic market.
pub fn synthetic_market(key: &str, floor_gp: f64, mint: Pubkey) -> Market {
    let lowalch = (floor_gp / 0.9).round().max(1.0) as u64;
    Market {
        key: key.to_string(),
        item: Item {
            obj_id: 1,
            cert_obj_id: None,
            name: key.to_string(),
            desc: String::new(),
            cost: lowalch * 5 / 2,
            lowalch,
            p0_gp_per_item: Some(floor_gp),
            symbol: key.to_uppercase(),
            stackable: false,
            members: false,
            mint: Some(mint.to_string()),
            pool: Some(Pubkey::new_unique().to_string()),
            flags: Vec::new(),
        },
        mint,
        pool: Pubkey::new_unique(),
    }
}

/// A whole market — registry entry, pool, history and position — ready to view.
pub struct TestMarket {
    pub market: Market,
    pub pool: PoolState,
    pub history: PriceHistory,
    pub position: Position,
    pub gp_available: f64,
    pub now: i64,
    pub supply: Option<f64>,
}

impl TestMarket {
    /// A pool trading at `premium` x its floor, with `floor_gp` as the floor.
    pub fn new(key: &str, floor_gp: f64, premium: f64) -> Self {
        let pool = synthetic_pool(floor_gp, premium, 100);
        let market = synthetic_market(key, floor_gp, pool.token_a_mint);
        Self {
            market,
            pool,
            history: PriceHistory::new(128),
            position: Position::default(),
            gp_available: 100_000.0,
            now: 1_787_000_000,
            supply: None,
        }
    }

    /// Set the item's on-chain token supply, as the engine would.
    pub fn with_supply(mut self, supply: f64) -> Self {
        self.supply = Some(supply);
        self
    }

    /// A lobster-sized pool sitting exactly on its floor.
    pub fn at_floor() -> Self {
        Self::new("lobster", 54.0, 1.0)
    }

    /// A lobster-sized pool trading above its floor.
    pub fn above_floor(premium: f64) -> Self {
        Self::new("lobster", 54.0, premium)
    }

    /// Give the book an opening position in this market.
    pub fn with_position(mut self, items: u64, avg_cost_gp: f64) -> Self {
        self.position = Position { items, avg_cost_gp };
        self
    }

    /// Seed the price history with `prices`, one per tick.
    pub fn with_history(mut self, prices: &[f64]) -> Self {
        for (i, price) in prices.iter().enumerate() {
            self.history
                .push(self.now - (prices.len() - i) as i64, *price);
        }
        self
    }

    /// Set the spendable GP the view reports.
    pub fn with_gp(mut self, gp: f64) -> Self {
        self.gp_available = gp;
        self
    }

    /// Borrow this market as a strategy sees it.
    pub fn view(&self) -> MarketView<'_> {
        MarketView {
            market: &self.market,
            pool: &self.pool,
            history: &self.history,
            position: self.position,
            now: self.now,
            current_point: self.now as u64,
            gp_available: self.gp_available,
            supply: self.supply,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_pools_price_and_quote_like_real_ones() {
        let market = TestMarket::above_floor(1.2);
        let view = market.view();
        assert!((view.floor() - 54.0).abs() / 54.0 < 1e-6);
        assert!((view.premium() - 1.2).abs() < 1e-6);
        let quote = view.quote_buy(5).expect("a 5-item buy should quote");
        // Buying costs a little more than spot: 1% fee plus impact.
        assert!(quote.execution_price > view.spot());
        assert!(quote.execution_price < view.spot() * 1.15);
    }

    #[test]
    fn a_pool_on_its_floor_has_no_bid_depth() {
        let view_owner = TestMarket::at_floor();
        let view = view_owner.view();
        assert!(view.exit_depth_gp() < 1.0);
        assert!(
            view.quote_sell(1).is_none(),
            "selling into a pool with no GP must be impossible, not merely unprofitable"
        );
    }

    #[test]
    fn depth_grows_as_the_pool_trades_up() {
        let shallow = TestMarket::above_floor(1.05);
        let deep = TestMarket::above_floor(1.5);
        assert!(deep.view().exit_depth_gp() > shallow.view().exit_depth_gp() * 5.0);
    }
}

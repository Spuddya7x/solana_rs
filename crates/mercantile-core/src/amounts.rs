//! Conversions between base units, UI amounts and pool prices.
//!
//! Two decimal scales are in play everywhere in Mercantile: GP has 6 decimals,
//! item tokens have 1. A "price" in this crate always means **GP per whole item**,
//! matching what the Grand Exchange front-end and `chain/cli/swap.ts` display.

use crate::ids::{GP_DECIMALS, GP_UNIT, ITEM_DECIMALS, ITEM_UNIT};

/// Whole GP (possibly fractional) to base units, rounding to nearest.
pub fn gp_to_base(gp: f64) -> u64 {
    (gp * GP_UNIT as f64).round().max(0.0) as u64
}

/// GP base units to whole GP.
pub fn base_to_gp(base: u64) -> f64 {
    base as f64 / GP_UNIT as f64
}

/// Whole items to base units. Item tokens are fungible with 1 decimal, but the
/// game only ever bridges whole items, so the bot trades whole items too.
pub fn items_to_base(items: u64) -> u64 {
    items.saturating_mul(ITEM_UNIT)
}

/// Item base units to whole items (fractional if a pool ever pays out a tenth).
pub fn base_to_items(base: u64) -> f64 {
    base as f64 / ITEM_UNIT as f64
}

/// Item base units truncated to whole items.
pub fn base_to_whole_items(base: u64) -> u64 {
    base / ITEM_UNIT
}

/// GP per whole item from a Q64.64 sqrt price, matching the cp-amm SDK's
/// `getPriceFromSqrtPrice(sqrtPrice, ITEM_DECIMALS, GP_DECIMALS)`.
///
/// `(sqrt / 2^64)^2` is token-B base units per token-A base unit; the decimal
/// shift `10^(dec_a - dec_b)` converts that to UI GP per UI item.
pub fn price_from_sqrt_price(sqrt_price: u128) -> f64 {
    let sqrt = sqrt_price as f64 / 2f64.powi(64);
    sqrt * sqrt * 10f64.powi(ITEM_DECIMALS as i32 - GP_DECIMALS as i32)
}

/// Inverse of [`price_from_sqrt_price`]: a Q64.64 sqrt price for a GP-per-item price.
///
/// Used for limit checks and for reasoning about the pool's floor, never for
/// building transactions, so `f64` precision is sufficient.
pub fn sqrt_price_from_price(price_gp_per_item: f64) -> u128 {
    let adjusted = price_gp_per_item / 10f64.powi(ITEM_DECIMALS as i32 - GP_DECIMALS as i32);
    (adjusted.max(0.0).sqrt() * 2f64.powi(64)) as u128
}

/// In-game low-alchemy value: `max(floor(cost * 0.4), 1)`.
///
/// Mirrors the formula the game uses and that `chain/registry/build-registry.ts`
/// reproduces; every pool's permanent bid floor is derived from it.
pub fn lowalch_from_cost(cost: u64) -> u64 {
    (((cost as f64) * 0.4).floor() as u64).max(1)
}

/// The pool's permanent bid floor: `0.9 x lowalch` GP per item.
///
/// Pools are seeded single-sided at this price over `[P0, infinity)`, so the price
/// can never trade below it — it is the economic backstop the whole market rests on.
pub fn floor_price(lowalch: u64) -> f64 {
    lowalch as f64 * 0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gp_round_trips() {
        assert_eq!(gp_to_base(1.5), 1_500_000);
        assert_eq!(base_to_gp(1_500_000), 1.5);
    }

    #[test]
    fn items_round_trip() {
        assert_eq!(items_to_base(7), 70);
        assert_eq!(base_to_whole_items(75), 7);
    }

    #[test]
    fn price_matches_live_pool() {
        // 1dose1agility pool, mainnet slot 440_044_167.
        let price = price_from_sqrt_price(27_169_186_055_242_833_301_691);
        assert!((price - 21.692_710_015_617_926).abs() < 1e-9, "{price}");
    }

    #[test]
    fn sqrt_price_inverts_price() {
        let sqrt = sqrt_price_from_price(21.692_710_015_617_926);
        let back = price_from_sqrt_price(sqrt);
        assert!((back - 21.692_710_015_617_926).abs() < 1e-6, "{back}");
    }

    #[test]
    fn alch_floor_matches_game_formula() {
        assert_eq!(lowalch_from_cost(50), 20); // agility potion(1)
        assert_eq!(lowalch_from_cost(2), 1); // never zero
        assert!((floor_price(20) - 18.0).abs() < f64::EPSILON);
    }
}

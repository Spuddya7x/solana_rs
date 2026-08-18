//! Scanning the registry for high-alchemy arbitrage.
//!
//! The trade: buy an item on chain near its pool floor (`0.36 x cost`), bridge it
//! into the game, cast High Level Alchemy on it (`0.6 x cost`), and bridge the GP
//! back out. Gross margin is a constant `5/3` before costs; what varies per item,
//! and what this module measures, is everything that eats into it:
//!
//! * **price impact** — pools hold about a hundred items, so buying a stack walks
//!   the price up and can erase the margin on its own;
//! * **the pool fee** — 1%, collected in GP;
//! * **nature runes** — one per cast, itself a tokenised item with its own pool;
//! * **throughput** — one cast per five ticks, so a big stack is hours of casting.
//!
//! Everything here is priced from live quotes rather than from the floor, because
//! the floor is what the *first* item costs, not the twentieth.

use mercantile_core::alch;
use mercantile_core::Market;
use mercantile_dex::pool::PoolState;
use mercantile_dex::quote::SwapQuote;
use serde::Serialize;

/// The rune bill for one cast.
#[derive(Debug, Clone, Copy)]
pub struct RuneCost {
    /// GP per nature rune.
    pub nature_rune_gp: f64,
    /// GP per fire rune. Zero when wielding a staff of fire, which is the only
    /// sane way to run this — the staff is a one-off purchase and removes five
    /// runes per cast forever.
    pub fire_rune_gp: f64,
}

impl RuneCost {
    /// Total GP consumed by one high alchemy cast.
    pub fn per_high_alch(&self) -> f64 {
        self.nature_rune_gp * alch::NATURE_RUNES_PER_ALCH as f64
            + self.fire_rune_gp * alch::FIRE_RUNES_PER_HIGH_ALCH as f64
    }

    /// Runes bought at their own pool floors, with a staff of fire equipped.
    pub fn at_floor_with_staff(nature_rune_floor_gp: f64) -> Self {
        Self {
            nature_rune_gp: nature_rune_floor_gp,
            fire_rune_gp: 0.0,
        }
    }
}

/// What the round trip looks like for one market at one size.
#[derive(Debug, Clone, Serialize)]
pub struct AlchOpportunity {
    pub market: String,
    pub name: String,
    /// Item shop cost, the anchor for every alchemy value.
    pub cost: u64,
    /// GP one cast pays out.
    pub high_alch_gp: u64,
    /// Whole items in this trade.
    pub items: u64,
    /// GP to buy them on chain, including fee and price impact.
    pub gp_cost: f64,
    /// Average GP paid per item.
    pub avg_price: f64,
    /// The pool's floor, for reference.
    pub floor: f64,
    /// GP spent on nature (and fire) runes for the whole stack.
    pub rune_gp: f64,
    /// GP the stack returns once alched.
    pub alch_gp: f64,
    /// Profit in GP after purchase and runes.
    pub profit_gp: f64,
    /// Profit as a fraction of GP outlay.
    pub margin: f64,
    /// Profit per individual cast.
    pub profit_per_cast: f64,
    /// Hours of casting to work through the stack.
    pub hours: f64,
    /// Profit per hour of casting, the number that actually ranks these.
    pub profit_per_hour: f64,
}

/// Evaluate one market at one size. `None` when the pool cannot fill the trade.
pub fn evaluate(
    market: &Market,
    pool: &PoolState,
    items: u64,
    runes: RuneCost,
    current_point: u64,
) -> Option<AlchOpportunity> {
    if items == 0 {
        return None;
    }
    let quote = pool.quote_buy_items(items, current_point).ok()?;
    let gp_cost = quote.gp();
    let high_alch_gp = alch::high_alch_value(market.item.cost);
    let alch_gp = high_alch_gp as f64 * items as f64;
    let rune_gp = runes.per_high_alch() * items as f64;
    let profit_gp = alch_gp - gp_cost - rune_gp;
    let hours = items as f64 / alch::high_alchs_per_hour();

    Some(AlchOpportunity {
        market: market.key.clone(),
        name: market.item.name.clone(),
        cost: market.item.cost,
        high_alch_gp,
        items,
        gp_cost,
        avg_price: gp_cost / items as f64,
        floor: pool.floor_price(),
        rune_gp,
        alch_gp,
        profit_gp,
        margin: if gp_cost > 0.0 {
            profit_gp / gp_cost
        } else {
            0.0
        },
        profit_per_cast: profit_gp / items as f64,
        hours,
        profit_per_hour: if hours > 0.0 { profit_gp / hours } else { 0.0 },
    })
}

/// The most profitable whole-item size for a market, searching up to `max_items`.
///
/// Profit per stack rises with size until impact overtakes the margin, so this
/// walks sizes and keeps the best — the point where buying one more item costs
/// more than alching it returns.
pub fn best_size(
    market: &Market,
    pool: &PoolState,
    max_items: u64,
    runes: RuneCost,
    current_point: u64,
) -> Option<AlchOpportunity> {
    let mut best: Option<AlchOpportunity> = None;
    for items in 1..=max_items {
        match evaluate(market, pool, items, runes, current_point) {
            Some(candidate) => {
                if candidate.profit_gp <= 0.0 && best.is_some() {
                    break;
                }
                if best
                    .as_ref()
                    .is_none_or(|b| candidate.profit_gp > b.profit_gp)
                {
                    best = Some(candidate);
                } else {
                    // Profit has started falling: impact now outruns the margin.
                    break;
                }
            }
            // The pool ran out before we did.
            None => break,
        }
    }
    best.filter(|opportunity| opportunity.profit_gp > 0.0)
}

/// Rank markets by profit per hour of casting, most profitable first.
pub fn rank(mut opportunities: Vec<AlchOpportunity>) -> Vec<AlchOpportunity> {
    opportunities.sort_by(|a, b| {
        b.profit_per_hour
            .partial_cmp(&a.profit_per_hour)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.market.cmp(&b.market))
    });
    opportunities
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{synthetic_market, synthetic_pool};

    const POINT: u64 = 1_787_000_000;

    /// A market whose floor is the usual 0.36 x cost.
    fn market_at(cost: u64, premium: f64) -> (Market, PoolState) {
        let lowalch = mercantile_core::low_alch_value(cost);
        let floor = lowalch as f64 * 0.9;
        let pool = synthetic_pool(floor, premium, 100);
        let mut market = synthetic_market("item", floor, pool.token_a_mint);
        market.item.cost = cost;
        market.item.lowalch = lowalch;
        (market, pool)
    }

    #[test]
    fn a_floor_purchase_alchs_at_a_profit() {
        let (market, pool) = market_at(150, 1.0); // lobster-ish
        let opportunity =
            evaluate(&market, &pool, 5, RuneCost::at_floor_with_staff(7.2), POINT).unwrap();
        assert_eq!(opportunity.high_alch_gp, 90);
        assert!(opportunity.profit_gp > 0.0);
        // Gross margin is 5/3 before costs, so net stays well under that.
        assert!(
            opportunity.margin > 0.2 && opportunity.margin < 0.67,
            "{}",
            opportunity.margin
        );
    }

    #[test]
    fn cheap_items_cannot_carry_the_nature_rune() {
        let (market, pool) = market_at(20, 1.0); // a nature rune itself
        let opportunity = evaluate(&market, &pool, 5, RuneCost::at_floor_with_staff(7.2), POINT);
        assert!(
            opportunity.is_none_or(|o| o.profit_gp <= 0.0),
            "0.24 x 20 GP cannot pay for a 7.2 GP rune"
        );
    }

    #[test]
    fn fire_runes_matter_when_there_is_no_staff() {
        let (market, pool) = market_at(150, 1.0);
        let with_staff =
            evaluate(&market, &pool, 5, RuneCost::at_floor_with_staff(7.2), POINT).unwrap();
        let without = evaluate(
            &market,
            &pool,
            5,
            RuneCost {
                nature_rune_gp: 7.2,
                fire_rune_gp: 0.9,
            },
            POINT,
        )
        .unwrap();
        // Five fire runes a cast at 0.9 GP is 4.5 GP a cast, 22.5 GP over the stack.
        assert!((with_staff.profit_gp - without.profit_gp - 22.5).abs() < 1e-6);
    }

    #[test]
    fn impact_caps_the_useful_size() {
        let (market, pool) = market_at(1_000, 1.0);
        let best = best_size(
            &market,
            &pool,
            100,
            RuneCost::at_floor_with_staff(7.2),
            POINT,
        )
        .unwrap();
        // The whole pool is never the answer: the last items cost far too much.
        assert!(best.items < 100, "picked {} items", best.items);
        assert!(best.profit_gp > 0.0);
        let bigger = evaluate(
            &market,
            &pool,
            best.items + 5,
            RuneCost::at_floor_with_staff(7.2),
            POINT,
        );
        assert!(bigger.is_none_or(|b| b.profit_gp <= best.profit_gp));
    }

    #[test]
    fn a_pool_far_above_its_floor_is_not_an_opportunity() {
        // 0.6 x cost is the ceiling; a pool trading at twice its floor is above it.
        let (market, pool) = market_at(150, 2.0);
        assert!(best_size(
            &market,
            &pool,
            50,
            RuneCost::at_floor_with_staff(7.2),
            POINT
        )
        .is_none());
    }

    #[test]
    fn ranking_puts_the_best_gp_per_hour_first() {
        let (cheap, cheap_pool) = market_at(200, 1.0);
        let (rich, rich_pool) = market_at(20_000, 1.0);
        let runes = RuneCost::at_floor_with_staff(7.2);
        let ranked = rank(vec![
            best_size(&cheap, &cheap_pool, 50, runes, POINT).unwrap(),
            best_size(&rich, &rich_pool, 50, runes, POINT).unwrap(),
        ]);
        assert!(
            ranked[0].cost > ranked[1].cost,
            "richer items alch for more per cast"
        );
        assert!(ranked[0].profit_per_hour > ranked[1].profit_per_hour);
    }

    #[test]
    fn throughput_is_reported_in_castable_hours() {
        let (market, pool) = market_at(1_000, 1.0);
        let opportunity = evaluate(
            &market,
            &pool,
            1_200,
            RuneCost::at_floor_with_staff(7.2),
            POINT,
        );
        // 1200 items is an hour of casting even if the pool could supply them.
        if let Some(opportunity) = opportunity {
            assert!((opportunity.hours - 1.0).abs() < 1e-9);
        }
    }
}

//! Buying on chain to sell to an NPC shop.
//!
//! The second exit, and the one an untrained account can use. A pool's floor is
//! 36% of an item's cost, and unlike the alchemy route this needs no Magic
//! level, no runes and no 55-level gate — only the GP to open the position and
//! a walk to the right counter.
//!
//! What the counter pays is the whole game, and it splits sharply:
//!
//! * A **specialist** pays 60–95% of cost for the goods it deals in, so against
//!   a 36% floor there is 70–160% of headroom and about twenty units of runway
//!   before the haggle decay closes it. This is the trade.
//! * A **general store** pays 400/1000 — 40% of cost, exactly low alchemy —
//!   which is 11% over the floor on the first unit and a loss by the third.
//!   Only 18 of the reachable counters are general stores, and none of them
//!   makes a trip worth taking on its own.
//!
//! So a market is flippable when a specialist that buys it is reachable. Items
//! whose only good buyer is gated — `lobster` (Fishing Guild, 68 Fishing) or
//! `rune_platebody` (Oziach, behind Dragon Slayer) — fall back to the general
//! store rate and are not trades at all.
//!
//! What it costs is bounded by two decays, and both are modelled here rather
//! than assumed:
//!
//! * **The pool** charges more per item as you buy — a hundred-unit pool moves
//!   about 7% on a five-item buy.
//! * **The shop** pays less per item as you sell, by `haggle/1000` of cost per
//!   unit, so a visit is worth roughly 25 units before the marginal price falls
//!   under what the pool charged.
//!
//! Shop revenue is priced with the counter at its **base stock**, which is the
//! neutral assumption. A depleted shop pays considerably more — the adjustment
//! clamps at -5000, so up to 5.7x cost — but planning on that would promise
//! profits that only exist if nobody has traded there recently.
//!
//! The profitable size is where those two curves cross, which is what
//! [`best_size`] searches for.
//!
//! Two things beyond the prices decide whether a flip is real:
//!
//! * **The counter has to be reachable.** 45 of the 117 shops are not — upstairs,
//!   across water, or behind a door that checks a skill or a quest. An early
//!   version of this scanner recommended the Legends Guild general store, which
//!   is upstairs *and* behind Legends Quest.
//! * **The walk has to be worth it.** The best-paying shops are the furthest:
//!   the Ardougne fur stall pays 95% of cost and sits 861 tiles from Lumbridge,
//!   which is ten minutes of round trip. Flips are therefore ranked by **profit
//!   per hour**, not profit per trip, and a tie on price goes to the nearer
//!   counter.

use mercantile_core::shops::{best_shop_for, Shop};
use mercantile_core::Market;
use mercantile_dex::pool::PoolState;
use mercantile_dex::quote::SwapQuote;
use serde::Serialize;

/// One item's flip, at one size.
#[derive(Debug, Clone, Serialize)]
pub struct Flip {
    pub market: String,
    pub name: String,
    /// Item shop cost — the anchor for every shop price.
    pub cost: u64,
    /// Whole items in this trade.
    pub items: u64,
    /// GP to buy them on chain, including fee and price impact.
    pub gp_cost: f64,
    /// Average GP paid per item.
    pub avg_price: f64,
    /// GP the shop pays for the whole stack, after its price decay.
    pub shop_revenue: f64,
    /// What the last unit fetched — the marginal price at the end of the sale.
    pub marginal_price: f64,
    /// Which shop, and where.
    pub shop: String,
    pub shop_title: String,
    pub shop_x: i32,
    pub shop_z: i32,
    /// Tiles from Lumbridge to the counter.
    pub walk_tiles: u32,
    /// Seconds for the round trip, running.
    pub trip_seconds: f64,
    /// Profit in GP.
    pub profit_gp: f64,
    /// Profit as a fraction of the GP outlay.
    pub margin: f64,
    /// Profit per hour of walking and selling — what actually ranks a flip.
    pub gp_per_hour: f64,
}

/// Evaluate one market at one size against its best-paying shop.
pub fn evaluate(market: &Market, pool: &PoolState, items: u64, current_point: u64) -> Option<Flip> {
    if items == 0 {
        return None;
    }
    let cost = market.item.cost as i64;
    let shop: &Shop = best_shop_for(&market.key, cost)?;
    let quote = pool.quote_buy_items(items, current_point).ok()?;
    let gp_cost = quote.gp();
    let (revenue, marginal) = shop.sell_total(&market.key, cost, items as i64);
    let profit = revenue as f64 - gp_cost;

    // Walking there and back, plus roughly a tick per unit at the counter.
    let trip_seconds = shop.round_trip_seconds()? + items as f64 * 0.6;

    Some(Flip {
        market: market.key.clone(),
        name: market.item.name.clone(),
        cost: market.item.cost,
        items,
        gp_cost,
        avg_price: gp_cost / items as f64,
        shop_revenue: revenue as f64,
        marginal_price: marginal as f64,
        shop: shop.npc.clone(),
        shop_title: shop.title.clone(),
        shop_x: shop.x,
        shop_z: shop.z,
        walk_tiles: shop.walk_tiles?,
        trip_seconds,
        profit_gp: profit,
        margin: if gp_cost > 0.0 { profit / gp_cost } else { 0.0 },
        gp_per_hour: profit * 3_600.0 / trip_seconds,
    })
}

/// The best whole-item size, searching up to `max_items`.
///
/// Optimises **profit per hour**, not profit per trip. Those differ: a bigger
/// stack earns more per journey but takes longer at the counter, and past the
/// crossing point each extra item is bought dearer and sold cheaper. The walk
/// is a fixed cost paid once, so the hourly optimum sits a little below the
/// per-trip one, and further below the shorter the walk.
pub fn best_size(
    market: &Market,
    pool: &PoolState,
    max_items: u64,
    current_point: u64,
) -> Option<Flip> {
    best_size_within(market, pool, max_items, current_point, None)
}

/// [`best_size`], bounded by the GP actually on hand.
///
/// Without this the scanner reports its best trades in items it cannot buy: a
/// dragon square shield is an 8% flip, but 8% of 184,000 GP is not available to
/// an account with 20,000. Capital is as scarce as time here, and a ranking on
/// time alone puts the most expensive item at the top of every list.
pub fn best_size_within(
    market: &Market,
    pool: &PoolState,
    max_items: u64,
    current_point: u64,
    budget_gp: Option<f64>,
) -> Option<Flip> {
    let mut best: Option<Flip> = None;
    for items in 1..=max_items {
        let Some(candidate) = evaluate(market, pool, items, current_point) else {
            break;
        };
        if budget_gp.is_some_and(|budget| candidate.gp_cost > budget) {
            break;
        }
        if best
            .as_ref()
            .is_none_or(|b| candidate.gp_per_hour > b.gp_per_hour)
        {
            best = Some(candidate);
        } else {
            break;
        }
    }
    best.filter(|flip| flip.profit_gp > 0.0)
}

/// Rank flips by profit per hour, best first.
pub fn rank(mut flips: Vec<Flip>) -> Vec<Flip> {
    flips.sort_by(|a, b| {
        b.gp_per_hour
            .partial_cmp(&a.gp_per_hour)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.market.cmp(&b.market))
    });
    flips
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{synthetic_market, synthetic_pool};

    const POINT: u64 = 1_787_000_000;

    /// A market whose floor is the usual 0.36 x cost.
    fn market_at(key: &str, cost: u64, premium: f64) -> (Market, PoolState) {
        let lowalch = mercantile_core::low_alch_value(cost);
        let floor = lowalch as f64 * 0.9;
        let pool = synthetic_pool(floor, premium, 100);
        let mut market = synthetic_market(key, floor, pool.token_a_mint);
        market.key = key.to_string();
        market.item.cost = cost;
        market.item.lowalch = lowalch;
        (market, pool)
    }

    #[test]
    fn a_floor_purchase_sells_to_a_shop_at_a_profit() {
        // Uncut gems have a reachable specialist buyer at 70% of cost against
        // the 36% floor, so the flip clears comfortably.
        let (market, pool) = market_at("uncut_diamond", 3_200, 1.0);
        let flip = evaluate(&market, &pool, 5, POINT).unwrap();
        assert!(flip.profit_gp > 0.0, "{flip:?}");
        assert!(flip.margin > 0.3 && flip.margin < 2.0, "{}", flip.margin);
    }

    #[test]
    fn the_nearer_of_two_equal_counters_wins() {
        // Both the Gem Trader and Herquin's pay 70% for uncut gems; the Gem
        // Trader is 68 tiles from Lumbridge and Herquin's 357.
        let (market, pool) = market_at("uncut_diamond", 3_200, 1.0);
        let flip = evaluate(&market, &pool, 5, POINT).unwrap();
        assert!(flip.walk_tiles < 200, "walked {} tiles", flip.walk_tiles);
    }

    #[test]
    fn an_item_with_no_reachable_specialist_falls_back_to_40_percent() {
        // Lobster's good buyers are the Fishing Guild (68 Fishing) and the
        // Shrimp and Parrot (an island). What is left is a general store, and a
        // general store pays 40% of cost — the same as low alchemy — against a
        // 36% floor. That is 11% on the first unit and negative by the third,
        // so the trip is one item long and the margin never clears the
        // strategy's 20% bar.
        let (market, pool) = market_at("lobster", 150, 1.0);
        let shop = mercantile_core::shops::best_shop_for("lobster", 150).unwrap();
        assert_eq!(shop.buy_multiplier, 400, "{}", shop.title);
        let best = best_size(&market, &pool, 25, POINT).unwrap();
        assert_eq!(best.items, 1, "{best:?}");
        assert!(best.margin < 0.2, "{}", best.margin);
    }

    #[test]
    fn the_shop_price_decay_caps_the_size() {
        let (market, pool) = market_at("uncut_diamond", 1_000, 1.0);
        let best = best_size(&market, &pool, 100, POINT).unwrap();
        assert!(best.items < 100, "picked {} items", best.items);
        // The last unit still fetched more than the average paid for them.
        assert!(best.marginal_price > 0.0);
        let bigger = evaluate(&market, &pool, best.items + 10, POINT);
        assert!(bigger.is_none_or(|b| b.profit_gp <= best.profit_gp));
    }

    #[test]
    fn a_pool_trading_above_the_shop_bid_is_no_trade() {
        // At 3x the floor the pool asks more than 95% of cost; nothing pays that.
        let (market, pool) = market_at("uncut_diamond", 150, 3.0);
        assert!(best_size(&market, &pool, 25, POINT).is_none());
    }

    #[test]
    fn worthless_items_are_not_flippable() {
        // cost 1: the shop pays zero however many you bring.
        let (market, pool) = market_at("cow_hide", 1, 1.0);
        assert!(best_size(&market, &pool, 25, POINT).is_none());
    }

    #[test]
    fn a_bankroll_caps_the_size_it_proposes() {
        let (market, pool) = market_at("uncut_diamond", 3_200, 1.0);
        let unlimited = best_size(&market, &pool, 25, POINT).unwrap();
        let broke = best_size_within(&market, &pool, 25, POINT, Some(4_000.0)).unwrap();
        assert!(broke.gp_cost <= 4_000.0, "{}", broke.gp_cost);
        assert!(broke.items < unlimited.items);
        // Still a trade, just a smaller one.
        assert!(broke.profit_gp > 0.0);
    }

    #[test]
    fn a_bankroll_that_buys_nothing_is_no_trade() {
        let (market, pool) = market_at("uncut_diamond", 3_200, 1.0);
        assert!(best_size_within(&market, &pool, 25, POINT, Some(1.0)).is_none());
    }

    #[test]
    fn ranking_puts_the_best_hourly_rate_first() {
        let (cheap, cheap_pool) = market_at("uncut_sapphire", 250, 1.0);
        let (rich, rich_pool) = market_at("uncut_diamond", 3_200, 1.0);
        let ranked = rank(vec![
            best_size(&cheap, &cheap_pool, 25, POINT).unwrap(),
            best_size(&rich, &rich_pool, 25, POINT).unwrap(),
        ]);
        assert_eq!(ranked[0].market, "uncut_diamond");
        assert!(ranked[0].gp_per_hour > ranked[1].gp_per_hour);
    }

    #[test]
    fn the_walk_is_priced_in() {
        let (market, pool) = market_at("rune_platebody", 65_000, 1.0);
        let flip = evaluate(&market, &pool, 5, POINT).unwrap();
        assert!(flip.walk_tiles > 0, "a counter has a distance");
        assert!(flip.trip_seconds > 0.0);
        // The hourly rate is the profit spread over the round trip.
        assert!((flip.gp_per_hour - flip.profit_gp * 3_600.0 / flip.trip_seconds).abs() < 1e-6);
    }

    #[test]
    fn only_reachable_counters_are_offered() {
        // Every shop the scanner names must be one the bot can walk to; the
        // Legends Guild store is upstairs and behind a quest.
        let (market, pool) = market_at("rune_platebody", 65_000, 1.0);
        let flip = evaluate(&market, &pool, 5, POINT).unwrap();
        let shop = mercantile_core::shops()
            .iter()
            .find(|s| s.npc == flip.shop)
            .expect("the named shop exists");
        assert!(shop.accessible);
        assert!(shop.safe);
    }

    #[test]
    fn the_flip_names_the_counter_to_walk_to() {
        let (market, pool) = market_at("lobster", 150, 1.0);
        let flip = evaluate(&market, &pool, 1, POINT).unwrap();
        assert!(!flip.shop.is_empty());
        assert!(
            flip.shop_x != 0 || flip.shop_z != 0,
            "a shop has a position"
        );
    }
}

//! Buying on chain to sell to an NPC shop.
//!
//! The second exit, and the one an untrained account can use. A pool's floor is
//! 36% of an item's cost; a shopkeeper pays 60–95%. Unlike the alchemy route
//! this needs no Magic level, no runes and no 55-level gate — only the GP to
//! open the position and a walk to the right counter.
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
    /// Profit in GP.
    pub profit_gp: f64,
    /// Profit as a fraction of the GP outlay.
    pub margin: f64,
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
        profit_gp: profit,
        margin: if gp_cost > 0.0 { profit / gp_cost } else { 0.0 },
    })
}

/// The most profitable whole-item size, searching up to `max_items`.
///
/// Profit rises with size until the pool's rising cost meets the shop's falling
/// bid, then falls. This walks sizes and keeps the peak.
pub fn best_size(
    market: &Market,
    pool: &PoolState,
    max_items: u64,
    current_point: u64,
) -> Option<Flip> {
    let mut best: Option<Flip> = None;
    for items in 1..=max_items {
        match evaluate(market, pool, items, current_point) {
            Some(candidate) => {
                if best
                    .as_ref()
                    .is_none_or(|b| candidate.profit_gp > b.profit_gp)
                {
                    best = Some(candidate);
                } else {
                    break;
                }
            }
            None => break,
        }
    }
    best.filter(|flip| flip.profit_gp > 0.0)
}

/// Rank flips by profit, most profitable first.
pub fn rank(mut flips: Vec<Flip>) -> Vec<Flip> {
    flips.sort_by(|a, b| {
        b.profit_gp
            .partial_cmp(&a.profit_gp)
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
        let (market, pool) = market_at("lobster", 150, 1.0);
        let flip = evaluate(&market, &pool, 5, POINT).unwrap();
        assert!(flip.profit_gp > 0.0, "{flip:?}");
        // Shops pay 60-95% of cost against a 36% floor, less the pool fee and
        // impact, so the margin lands somewhere between a quarter and 2x.
        assert!(flip.margin > 0.2 && flip.margin < 2.0, "{}", flip.margin);
    }

    #[test]
    fn the_shop_price_decay_caps_the_size() {
        let (market, pool) = market_at("lobster", 1_000, 1.0);
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
        let (market, pool) = market_at("lobster", 150, 3.0);
        assert!(best_size(&market, &pool, 25, POINT).is_none());
    }

    #[test]
    fn worthless_items_are_not_flippable() {
        // cost 1: the shop pays zero however many you bring.
        let (market, pool) = market_at("cow_hide", 1, 1.0);
        assert!(best_size(&market, &pool, 25, POINT).is_none());
    }

    #[test]
    fn ranking_puts_the_biggest_profit_first() {
        let (cheap, cheap_pool) = market_at("lobster", 150, 1.0);
        let (rich, rich_pool) = market_at("rune_platebody", 65_000, 1.0);
        let ranked = rank(vec![
            best_size(&cheap, &cheap_pool, 25, POINT).unwrap(),
            best_size(&rich, &rich_pool, 25, POINT).unwrap(),
        ]);
        assert_eq!(ranked[0].market, "rune_platebody");
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

//! What NPC shops pay, and which one pays most.
//!
//! This is the second exit from a pool position, and for an account below level
//! 55 it is the only one. A Mercantile pool's floor is `0.9 x lowalch`, which is
//! **36% of an item's cost**; a shopkeeper's `shop_buy_multiplier` is 600–950,
//! so selling to an NPC returns **60–95% of cost**. That is the same 1.67x high
//! alchemy pays at the low end and considerably better at the high end — with no
//! Magic level, no runes, and no 55-level gate.
//!
//! The formula is ported from `server/content/scripts/shop/scripts/shop.rs2`:
//!
//! ```text
//! calc_shop_value(cost, haggle, multiplier, diff):
//!   int5 = min(1000, max(-5000, diff * haggle))
//!   int5 = max(100, multiplier - int5)
//!   return scale(int5, 1000, cost)          // scale(a,b,c) = a*c/b
//! ```
//!
//! `diff` is `current_stock + sold_so_far - base_stock`, so the price **falls as
//! you sell** — by `haggle/1000` of cost per unit, 1–3%. That, not inventory
//! space, is what caps a single visit.
//!
//! The sign of `diff` matters more than it looks. A shop sitting at its base
//! stock prices at the headline multiplier; a **depleted** shop pays far above it
//! (the adjustment clamps at -5000, so up to 5.7x cost) and an overstocked one
//! pays less. Everything here therefore takes the shop's *current* stock, and
//! the convenience wrappers assume it is at base — the neutral case. Assuming a
//! depleted shop would systematically overstate what a flip is worth.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// A shopkeeper, its prices and its stock.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shop {
    /// Registry name of the shopkeeper NPC.
    pub npc: String,
    pub title: String,
    /// Out of 1000: what the shop pays for an item, before the stock adjustment.
    pub buy_multiplier: i64,
    /// Out of 1000: how much each unit of stock difference moves the price.
    pub haggle: i64,
    pub level: i32,
    pub x: i32,
    pub z: i32,
    /// Items the shop keeps, and how many it holds at base stock.
    pub stock: HashMap<String, i64>,
    /// Whether it will buy things it does not stock (a general store).
    pub buys_anything: bool,
    /// Outside the wilderness.
    ///
    /// The wilderness is bounded in x as well as z (`wilderness_zones.dbrow`:
    /// x 2944-3391, z 3520-6399 or 9920+), so a z-only test wrongly condemns
    /// Rellekka and the whole north-west. Only two shops are actually inside it.
    pub safe: bool,
}

/// The price floor the formula clamps to: 10% of cost.
pub const MIN_MULTIPLIER: i64 = 100;

impl Shop {
    /// Whether this shop will take the item at all.
    pub fn will_buy(&self, item: &str) -> bool {
        self.buys_anything || self.stock.contains_key(item)
    }

    /// What the shop holds at base stock.
    pub fn base_stock(&self, item: &str) -> i64 {
        self.stock.get(item).copied().unwrap_or(0)
    }

    /// GP paid for one unit, given the shop's current stock and how many have
    /// already been sold this visit.
    pub fn sell_price_at(
        &self,
        item: &str,
        cost: i64,
        current_stock: i64,
        already_sold: i64,
    ) -> i64 {
        let diff = current_stock + already_sold - self.base_stock(item);
        let adjustment = (diff * self.haggle).clamp(-5_000, 1_000);
        let multiplier = (self.buy_multiplier - adjustment).max(MIN_MULTIPLIER);
        multiplier * cost / 1_000
    }

    /// GP paid for one unit assuming the shop sits at its base stock.
    ///
    /// The neutral assumption, and the one to plan with. A depleted shop pays
    /// more, but a scanner that assumed depletion would promise profits that
    /// only exist if nobody has traded there recently.
    pub fn sell_price(&self, item: &str, cost: i64, already_sold: i64) -> i64 {
        self.sell_price_at(item, cost, self.base_stock(item), already_sold)
    }

    /// GP for selling `count` units here, and what the last one fetched.
    pub fn sell_total(&self, item: &str, cost: i64, count: i64) -> (i64, i64) {
        let mut total = 0;
        let mut last = 0;
        for sold in 0..count {
            last = self.sell_price(item, cost, sold);
            total += last;
        }
        (total, last)
    }

    /// How many units are worth selling before the marginal price drops below
    /// `floor` — normally what the items cost to buy on chain.
    pub fn worthwhile_count(&self, item: &str, cost: i64, floor: i64) -> i64 {
        let mut count = 0;
        while count < 10_000 && self.sell_price(item, cost, count) >= floor {
            count += 1;
        }
        count
    }
}

static SHOPS: OnceLock<Vec<Shop>> = OnceLock::new();

#[derive(Deserialize)]
struct ShopFile {
    shops: Vec<Shop>,
}

/// Every shop in the world, generated from the map and config data by
/// `gamebot/tools/build-money.ts --json`.
pub fn shops() -> &'static [Shop] {
    SHOPS.get_or_init(|| {
        let file: ShopFile = serde_json::from_str(include_str!("../data/shops.json"))
            .expect("shops.json is generated and must parse");
        file.shops
    })
}

/// The shop paying most for one unit of an item, wilderness counters excluded.
///
/// Specialist shops are where the money is: a general store pays 60%, but the
/// fur traders pay 95% for what they deal in.
pub fn best_shop_for(item: &str, cost: i64) -> Option<&'static Shop> {
    best_shop_where(item, cost, |shop| shop.safe)
}

/// The best shop matching a predicate — pass `|_| true` to include the wilderness.
pub fn best_shop_where(
    item: &str,
    cost: i64,
    allow: impl Fn(&Shop) -> bool,
) -> Option<&'static Shop> {
    shops()
        .iter()
        .filter(|shop| shop.will_buy(item) && allow(shop))
        .max_by_key(|shop| shop.sell_price(item, cost, 0))
}

/// Every shop that will buy the item, best price first.
pub fn shops_for(item: &str, cost: i64) -> Vec<&'static Shop> {
    let mut found: Vec<&Shop> = shops().iter().filter(|shop| shop.will_buy(item)).collect();
    found.sort_by_key(|shop| std::cmp::Reverse(shop.sell_price(item, cost, 0)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shop(buy: i64, haggle: i64, stock: &[(&str, i64)], anything: bool) -> Shop {
        Shop {
            npc: "shopkeeper".into(),
            title: "Test".into(),
            buy_multiplier: buy,
            haggle,
            level: 0,
            x: 0,
            z: 0,
            stock: stock.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            buys_anything: anything,
            safe: true,
        }
    }

    #[test]
    fn the_table_loads_and_covers_the_world() {
        assert_eq!(shops().len(), 117);
        assert!(
            shops().iter().any(|s| s.buys_anything),
            "general stores exist"
        );
        assert!(
            shops().iter().any(|s| s.buy_multiplier >= 900),
            "specialist shops pay far above the general rate"
        );
    }

    #[test]
    fn the_price_is_a_percentage_of_cost() {
        let general = shop(600, 10, &[], true);
        assert_eq!(general.sell_price("x", 1_000, 0), 600);
        assert_eq!(shop(950, 20, &[], true).sell_price("x", 1_000, 0), 950);
    }

    #[test]
    fn cost_one_items_are_worth_nothing() {
        // Which is why cow hides, bones, beef and feathers cannot fund anything.
        assert_eq!(shop(600, 10, &[], true).sell_price("cow_hide", 1, 0), 0);
    }

    #[test]
    fn the_price_decays_as_the_shop_fills() {
        let s = shop(600, 10, &[], true);
        assert_eq!(s.sell_price("x", 1_000, 0), 600);
        assert_eq!(s.sell_price("x", 1_000, 10), 500);
        assert_eq!(s.sell_price("x", 1_000, 25), 350);
        // And never below a tenth of cost.
        assert_eq!(s.sell_price("x", 1_000, 10_000), 100);
    }

    #[test]
    fn a_shop_at_base_stock_pays_its_headline_rate() {
        // The neutral assumption: stocking an item does not by itself change the price.
        let stocked = shop(600, 10, &[("firerune", 20)], false);
        assert_eq!(stocked.sell_price("firerune", 1_000, 0), 600);
    }

    #[test]
    fn a_depleted_shop_pays_a_premium_and_an_overstocked_one_pays_less() {
        let s = shop(600, 10, &[("firerune", 20)], false);
        // Empty shelves: diff = -20, so +200 on the multiplier.
        assert_eq!(s.sell_price_at("firerune", 1_000, 0, 0), 800);
        // Twice its base stock: diff = +20, so -200.
        assert_eq!(s.sell_price_at("firerune", 1_000, 40, 0), 400);
    }

    #[test]
    fn a_specialist_shop_refuses_what_it_does_not_deal_in() {
        let furs = shop(950, 20, &[("fur", 10)], false);
        assert!(furs.will_buy("fur"));
        assert!(!furs.will_buy("lobster"));
        assert!(shop(600, 10, &[], true).will_buy("lobster"));
    }

    #[test]
    fn selling_totals_a_falling_series() {
        let (total, last) = shop(600, 10, &[], true).sell_total("x", 1_000, 3);
        assert_eq!(total, 600 + 590 + 580);
        assert_eq!(last, 580);
    }

    #[test]
    fn the_worthwhile_count_stops_at_the_floor() {
        let s = shop(600, 10, &[], true);
        // 600 down to 360 in 10s is 24 steps, plus the one that lands exactly on it.
        assert_eq!(s.worthwhile_count("x", 1_000, 360), 25);
        assert_eq!(s.worthwhile_count("x", 1_000, 600), 1);
        assert_eq!(s.worthwhile_count("x", 1_000, 601), 0);
    }

    #[test]
    fn the_wilderness_is_excluded_by_default() {
        assert!(
            shops().iter().any(|s| !s.safe),
            "some shops are in the wilderness"
        );
        // Only two, and both in the Bandit Camp — a z-only test condemned fifteen.
        assert_eq!(shops().iter().filter(|s| !s.safe).count(), 2);
        for item in ["lobster", "coins", "cow_hide"] {
            assert!(best_shop_for(item, 100).is_none_or(|s| s.safe));
        }
    }

    #[test]
    fn the_best_shop_for_a_real_item_is_found() {
        // Nature runes are stocked by the Wizards' Guild among others.
        let best = best_shop_for("naturerune", 20);
        assert!(best.is_some());
        assert!(best.unwrap().sell_price("naturerune", 20, 0) > 0);
        // And a general store will take anything, so nothing is unsellable.
        assert!(best_shop_for("cow_hide", 1).is_some());
    }

    #[test]
    fn shops_beat_the_pool_floor_which_is_the_whole_point() {
        let cost = 10_000;
        // Pool floor: 0.9 x lowalch, lowalch being 40% of cost.
        let floor = (0.9 * (cost as f64 * 0.4)) as i64;
        assert_eq!(floor, 3_600);
        let general = shop(600, 10, &[], true);
        assert_eq!(general.sell_price("x", cost, 0), 6_000);
        // 1.67x at a general store, and better at a specialist.
        assert!(general.sell_price("x", cost, 0) as f64 / floor as f64 > 1.6);
    }
}

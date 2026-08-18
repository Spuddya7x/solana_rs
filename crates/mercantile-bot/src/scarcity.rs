//! Ranking markets by how little of the item can ever exist.
//!
//! The alch loop's output is GP, and GP is the worst asset in this economy: 10
//! billion supply, plus whatever alchemy mints from nothing. Converting it into
//! something whose supply cannot grow is the only way the work compounds rather
//! than dilutes.
//!
//! Scarcity here is measurable, not asserted, and rests on two facts:
//!
//! * **Every pool was seeded with exactly 100 units** (`SEED_PER_ITEM = 100n`),
//!   so any supply beyond 100 was bridged out of the game by a player. That
//!   number is direct evidence of how farmable an item is.
//! * **62 registry items have no in-game source at all** — no drop table, no
//!   shop, no skill, no quest, no ground spawn — so their supply cannot grow.
//!   See [`mercantile_core::sources`].
//!
//! Live supply beats the static table where they disagree: an item the table
//! calls capped but which has 99 units bridged in is being produced somehow, and
//! the evidence wins.

use mercantile_core::sources::{sources_for, ItemSources, POOL_SEED_UNITS};
use mercantile_core::Market;
use mercantile_dex::pool::PoolState;
use mercantile_dex::quote::SwapQuote;
use serde::Serialize;

/// What is known about one market's scarcity.
#[derive(Debug, Clone, Serialize)]
pub struct Scarcity {
    pub market: String,
    pub name: String,
    /// Total tokens in existence, in whole items.
    pub supply: f64,
    /// Supply above the 100-unit genesis seed: what players have bridged out.
    pub bridged_in: f64,
    /// Where the game says the item comes from.
    pub sources: ItemSources,
    /// Whether nothing in the game produces it *and* nothing has been bridged in.
    pub capped: bool,
    /// Current pool price in GP per item.
    pub price: f64,
    /// The pool's permanent floor.
    pub floor: f64,
    /// GP to buy the requested size, including fee and impact.
    pub gp_cost: Option<f64>,
    /// Whole items that trade would buy.
    pub items: u64,
    /// 0 to 1, higher being scarcer. See [`Scarcity::score`].
    pub score: f64,
}

impl Scarcity {
    /// A single number for ranking, in `[0, 1]`.
    ///
    /// Weighted towards the thing that cannot be undone: an item with no way
    /// into the game can never be diluted, whatever its price does. Bridged-in
    /// supply is the counterweight — it is proof of production, so it discounts
    /// the score smoothly rather than flipping a flag.
    fn compute_score(capped: bool, bridged_in: f64, supply: f64) -> f64 {
        // No in-game source is worth most of the score on its own.
        let source_score = if capped { 0.7 } else { 0.0 };
        // Everything bridged in is dilution: 0 extra units scores 0.3, and the
        // score decays towards 0 as production ramps up. 100 bridged units — a
        // doubling of supply — costs about two thirds of this component.
        let dilution_score = 0.3 / (1.0 + bridged_in / 50.0);
        // A very large supply is its own disqualifier regardless of provenance.
        let supply_penalty = if supply > 10_000.0 { 0.5 } else { 1.0 };
        ((source_score + dilution_score) * supply_penalty).clamp(0.0, 1.0)
    }

    /// GP per unit of scarcity — what accumulating actually costs.
    pub fn gp_per_score(&self) -> Option<f64> {
        let cost = self.gp_cost?;
        (self.score > 0.0).then(|| cost / self.score)
    }
}

/// Assess one market. `supply` is the live token supply in whole items.
pub fn assess(
    market: &Market,
    pool: &PoolState,
    supply: f64,
    items_to_price: u64,
    current_point: u64,
) -> Scarcity {
    let sources = sources_for(&market.key);
    let bridged_in = (supply - POOL_SEED_UNITS as f64).max(0.0);
    // The table says nothing produces it; the chain says nobody has produced it.
    // Both must hold.
    let capped = sources.capped && bridged_in == 0.0;
    let quote = pool.quote_buy_items(items_to_price, current_point).ok();

    Scarcity {
        market: market.key.clone(),
        name: market.item.name.clone(),
        supply,
        bridged_in,
        sources,
        capped,
        price: pool.spot_price(),
        floor: pool.floor_price(),
        gp_cost: quote.as_ref().map(|q| q.gp()),
        items: items_to_price,
        score: Scarcity::compute_score(capped, bridged_in, supply),
    }
}

/// Rank by scarcity, most scarce first, tie-broken by the pool floor.
///
/// The tie-break matters more than it looks. Scarcity on its own is not an
/// objective: 40 of the 62 items with no in-game source are macro-event cubes,
/// quest flowers and a cooking pot, all floored at 0.9 GP and wanted by nobody.
/// Ordering ties by *cheapest* — the obvious choice — puts exactly that junk at
/// the top of the list. The pool floor is the protocol's own statement of what
/// an item is worth (the rares were deliberately repriced to 10M GP), so among
/// equally unprintable items, the valuable one wins.
pub fn rank(mut rows: Vec<Scarcity>) -> Vec<Scarcity> {
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.floor
                    .partial_cmp(&a.floor)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.market.cmp(&b.market))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{synthetic_market, synthetic_pool};

    const POINT: u64 = 1_787_000_000;

    fn market(key: &str, floor: f64) -> (Market, PoolState) {
        let pool = synthetic_pool(floor, 1.1, 100);
        let mut m = synthetic_market(key, floor, pool.token_a_mint);
        m.key = key.to_string();
        (m, pool)
    }

    #[test]
    fn an_untouched_rare_scores_highest() {
        let (m, pool) = market("red_partyhat", 10_000_000.0);
        let rare = assess(&m, &pool, 100.0, 1, POINT);
        assert!(rare.capped);
        assert_eq!(rare.bridged_in, 0.0);
        assert!(rare.score > 0.9, "{}", rare.score);
    }

    #[test]
    fn a_farmable_item_scores_low_however_expensive_it_is() {
        let (m, pool) = market("rune_platebody", 23_400.0);
        let farmable = assess(&m, &pool, 100.0, 1, POINT);
        assert!(!farmable.capped);
        assert!(farmable.score < 0.35, "{}", farmable.score);
        assert!(farmable.sources.describe().contains("shops"));
    }

    #[test]
    fn evidence_of_production_beats_the_static_table() {
        let (m, pool) = market("blue_partyhat", 10_000_000.0);
        // The table calls it capped, but 99 units were bridged in from somewhere.
        let observed = assess(&m, &pool, 199.0, 1, POINT);
        assert!(observed.sources.capped, "the table still says so");
        assert!(!observed.capped, "but the chain says otherwise");
        assert!(observed.score < 0.2, "{}", observed.score);
    }

    #[test]
    fn dilution_decays_the_score_smoothly() {
        let (m, pool) = market("christmas_cracker", 10_000_000.0);
        let scores: Vec<f64> = [100.0, 110.0, 150.0, 300.0]
            .iter()
            .map(|supply| assess(&m, &pool, *supply, 1, POINT).score)
            .collect();
        for pair in scores.windows(2) {
            assert!(
                pair[0] > pair[1],
                "score must fall as supply grows: {scores:?}"
            );
        }
    }

    #[test]
    fn ranking_puts_scarcity_first_and_value_second() {
        let (cheap, cheap_pool) = market("red_partyhat", 100.0);
        let (dear, dear_pool) = market("purple_partyhat", 10_000.0);
        let (common, common_pool) = market("lobster", 54.0);
        let ranked = rank(vec![
            assess(&cheap, &cheap_pool, 100.0, 1, POINT),
            assess(&common, &common_pool, 100.0, 1, POINT),
            assess(&dear, &dear_pool, 100.0, 1, POINT),
        ]);
        assert_eq!(
            ranked[0].market, "purple_partyhat",
            "equally scarce, worth more"
        );
        assert_eq!(ranked[1].market, "red_partyhat");
        assert_eq!(ranked[2].market, "lobster", "farmable ranks last");
    }

    #[test]
    fn worthless_but_unprintable_items_do_not_outrank_rares() {
        // 40 of the 62 capped items are macro cubes and quest litter at the
        // 0.9 GP floor. Scarcity is necessary, not sufficient.
        let (cube, cube_pool) = market("macro_cube_bluestar", 0.9);
        let (hat, hat_pool) = market("red_partyhat", 10_000_000.0);
        let ranked = rank(vec![
            assess(&cube, &cube_pool, 100.0, 1, POINT),
            assess(&hat, &hat_pool, 100.0, 1, POINT),
        ]);
        assert_eq!(ranked[0].market, "red_partyhat");
    }

    #[test]
    fn gp_per_score_prices_the_scarcity() {
        let (m, pool) = market("red_partyhat", 1_000.0);
        let rare = assess(&m, &pool, 100.0, 1, POINT);
        let per = rare.gp_per_score().unwrap();
        assert!((per - rare.gp_cost.unwrap() / rare.score).abs() < 1e-9);

        let (m, pool) = market("lobster", 54.0);
        let common = assess(&m, &pool, 5_000.0, 1, POINT);
        // Heavily produced, so cheap scarcity is not on offer at any price.
        assert!(common.gp_per_score().unwrap() > per / 10.0);
    }
}

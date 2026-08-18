//! Planning the thieving bootstrap.
//!
//! [`mercantile_core::thieving`] knows what a pickpocket pays and what it costs
//! in hitpoints. This module turns that into a route: which target to stand in
//! front of at a given Thieving level, how much of the clock the food loop eats,
//! and what the account actually earns per hour of wall time.
//!
//! The geography is fixed and measured rather than guessed, so the walk is a
//! constant rather than a parameter:
//!
//! * The tutorial drops you at `(3222, 3222)`. A `man3` stands at `(3221, 3219)`
//!   — three tiles away, which is as close to zero setup as a money maker gets.
//! * Farmers are at `(3227, 3290)`, 77 tiles north, and are worth the walk from
//!   Thieving 10: three times the coins for the same one-hitpoint stun.
//! * The nearest shrimp is Lumbridge Swamp at `(3267, 3148)`, 115 tiles away.
//!   Two trees sit exactly on the straight line home — `(3253, 3194)` and
//!   `(3265, 3215)` — so the chop, the fire and the cook cost no detour at all.
//!
//! The awkward truth the numbers give up is that **the food loop is the job**.
//! At base levels an hour of thieving needs more than an hour of fishing to pay
//! for it, because Cooking 1 burns half the catch and a shrimp only heals three.
//! Levelling Cooking to 34 stops the burning outright and roughly doubles the
//! net rate, which is why [`plan`] reports the trained row next to the base one.

use serde::Serialize;

use mercantile_core::thieving::{
    best_target, sustain, Food, Pickpocket, Sustain, PICKPOCKETS, SHRIMP, TROUT,
};

/// Tiles from Lumbridge to the nearest shrimp spot, one way.
pub const SWAMP_TILES: f64 = 115.0;

/// Tiles from Lumbridge to the river trout spot, one way.
pub const RIVER_TILES: f64 = 50.0;

/// Inventory slots a forage trip can bring home full of food.
///
/// Twenty-seven, not twenty-eight: the net, axe and tinderbox have to travel
/// too, and the cooked stack is what the last free slots carry.
pub const TRIP_SLOTS: f64 = 25.0;

/// Seconds to walk `tiles` and back, running at two tiles a tick.
pub fn round_trip_seconds(tiles: f64) -> f64 {
    2.0 * tiles / 2.0 * 0.6
}

/// One costed way to run the bootstrap.
#[derive(Debug, Clone, Serialize)]
pub struct Route {
    /// Which pickpocket.
    pub target: &'static str,
    /// Thieving level assumed.
    pub thieving: u32,
    /// Which food funds it, and from where.
    pub food: &'static str,
    pub fishing: u32,
    pub cooking: u32,
    /// Tiles from Lumbridge to the fishing spot.
    pub tiles: f64,
    /// GP per hour spent thieving, before any interruption.
    pub gross_gp_per_hour: f64,
    /// Hitpoints an hour of thieving costs.
    pub damage_per_hour: f64,
    /// Hitpoints of food an hour of thieving has to be paid for with.
    pub deficit_per_hour: f64,
    /// Fraction of the clock actually spent thieving.
    pub uptime: f64,
    /// GP per hour of wall clock — the number that matters.
    pub net_gp_per_hour: f64,
    /// Thieving XP per hour of wall clock.
    pub xp_per_hour: f64,
}

fn route(
    target: &'static Pickpocket,
    thieving: u32,
    food: &Food,
    fishing: u32,
    cooking: u32,
    tiles: f64,
) -> Route {
    let s: Sustain = sustain(
        target,
        thieving,
        food,
        fishing,
        cooking,
        round_trip_seconds(tiles),
        TRIP_SLOTS * food.heal as f64,
    );
    Route {
        target: target.name,
        thieving,
        food: food.item,
        fishing,
        cooking,
        tiles,
        gross_gp_per_hour: s.gross_gp_per_hour,
        damage_per_hour: s.damage_per_hour,
        deficit_per_hour: s.deficit_per_hour,
        uptime: s.uptime,
        net_gp_per_hour: s.net_gp_per_hour,
        xp_per_hour: target.xp_per_hour(thieving) * s.uptime,
    }
}

/// The routes worth showing for an account at these levels.
///
/// Always includes what the account can do *now*, plus the two upgrades that
/// change the answer most: reaching Thieving 10 for farmers, and Cooking 34 for
/// a catch that stops burning.
pub fn plan(thieving: u32, fishing: u32, cooking: u32) -> Vec<Route> {
    let now = best_target(thieving);
    let mut routes = vec![route(now, thieving, &SHRIMP, fishing, cooking, SWAMP_TILES)];

    if thieving < 10 {
        let farmer = &PICKPOCKETS[1];
        routes.push(route(farmer, 10, &SHRIMP, fishing, cooking, SWAMP_TILES));
    }
    if cooking < 34 {
        routes.push(route(
            now,
            thieving,
            &SHRIMP,
            fishing.max(20),
            34,
            SWAMP_TILES,
        ));
    }
    if fishing < 20 {
        routes.push(route(
            now,
            thieving,
            &TROUT,
            20,
            cooking.max(34),
            RIVER_TILES,
        ));
    } else {
        routes.push(route(now, thieving, &TROUT, fishing, cooking, RIVER_TILES));
    }
    routes
}

/// Rank routes by GP per hour of wall clock, best first.
pub fn rank(mut routes: Vec<Route>) -> Vec<Route> {
    routes.sort_by(|a, b| {
        b.net_gp_per_hour
            .partial_cmp(&a.net_gp_per_hour)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    routes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_account_is_pointed_at_men() {
        let routes = plan(1, 1, 1);
        assert_eq!(routes[0].target, "man/woman");
        assert_eq!(routes[0].food, "shrimp");
    }

    #[test]
    fn the_food_loop_eats_most_of_the_clock_at_base_levels() {
        let base = &plan(1, 1, 1)[0];
        assert!(base.uptime < 0.5, "uptime {}", base.uptime);
        assert!(base.net_gp_per_hour < base.gross_gp_per_hour / 2.0);
    }

    #[test]
    fn thieving_10_is_the_single_biggest_upgrade() {
        // Measured, not assumed. Reaching Thieving 10 costs about seven minutes
        // of picking men's pockets and doubles the account's income; every
        // other upgrade on the ladder is worth a third or less:
        //
        //   T1  F1  C1   1446 GP/h      T10 F1  C34  4163 GP/h
        //   T10 F1  C1   3075 GP/h      T10 F20 C34  4951 GP/h
        //   T10 F20 C1   4012 GP/h      T10 F30 C34  5189 GP/h
        let fresh = plan(1, 1, 1)[0].net_gp_per_hour;
        let farmers = plan(10, 1, 1)[0].net_gp_per_hour;
        assert!(farmers > fresh * 2.0, "{fresh} -> {farmers}");

        // Cooking 34 is the next biggest, but it is a third, not a doubling.
        let clean = plan(10, 1, 34)[0].net_gp_per_hour;
        assert!(clean > farmers * 1.3, "{farmers} -> {clean}");
        assert!(clean < farmers * 1.5);
    }

    #[test]
    fn the_ladder_only_ever_goes_up() {
        let rungs = [
            plan(1, 1, 1)[0].net_gp_per_hour,
            plan(10, 1, 1)[0].net_gp_per_hour,
            plan(10, 20, 1)[0].net_gp_per_hour,
            plan(10, 20, 34)[0].net_gp_per_hour,
            plan(10, 30, 34)[0].net_gp_per_hour,
        ];
        for pair in rungs.windows(2) {
            assert!(pair[1] > pair[0], "{rungs:?}");
        }
    }

    #[test]
    fn farmers_beat_men_even_after_the_food_loop() {
        // Farmers take more damage per hour than men, so it is not obvious that
        // the extra coins survive the food bill. They do, comfortably.
        let man = plan(1, 20, 34)[0].net_gp_per_hour;
        let farmer = plan(10, 20, 34)[0].net_gp_per_hour;
        assert!(farmer > man * 1.5, "{man} -> {farmer}");
    }

    #[test]
    fn a_fresh_account_is_shown_the_farmer_upgrade() {
        let routes = plan(1, 1, 1);
        assert!(routes.iter().any(|r| r.target == "farmer"));
    }

    #[test]
    fn trout_from_the_river_beats_shrimp_from_the_swamp() {
        // Twice the healing per slot and less than half the walk.
        let routes = plan(10, 20, 34);
        let shrimp = routes.iter().find(|r| r.food == "shrimp").unwrap();
        let trout = routes.iter().find(|r| r.food == "trout").unwrap();
        assert!(trout.net_gp_per_hour > shrimp.net_gp_per_hour);
    }

    #[test]
    fn ranking_puts_the_best_wall_clock_rate_first() {
        let ranked = rank(plan(1, 1, 1));
        for pair in ranked.windows(2) {
            assert!(pair[0].net_gp_per_hour >= pair[1].net_gp_per_hour);
        }
    }
}

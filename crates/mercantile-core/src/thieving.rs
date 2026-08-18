//! Pickpocketing, and the healing bill that comes with it.
//!
//! Thieving is the only money maker that needs **no capital, no levels and no
//! equipment**: a man stands two tiles from where the tutorial drops you, and
//! picking his pocket pays three coins. That makes it the natural bootstrap for
//! an account with nothing, which is what the rest of the suite assumes it has.
//!
//! Every number here is read out of the engine rather than remembered:
//!
//! * The table is `skill_thieving/configs/pickpocking/pickpocket.dbrow`.
//! * The roll is `STAT_RANDOM` in `PlayerOps.ts`, reproduced in [`stat_random`].
//! * The failure branch is `~fail_pick_pocket` in `thieving.rs2`: it deals
//!   `stun_damage` and sets `%action_delay` to `map_clock + stun_ticks`.
//! * Healing is 1 HP per 100 ticks (`settimer(health_regen, 100)` at login) plus
//!   whatever you eat; food heals come from `consume_normal.dbrow`.
//!
//! Two consequences shape every plan built on this:
//!
//! * **The loot roll is not random.** `pick_pocket_check_for_reward` rolls
//!   `random(128)` against a denominator that the man's single `128`-weight
//!   entry drives straight to zero, so a success always pays exactly its coins.
//!   All the variance is in whether you succeed at all.
//! * **Failure costs eight ticks and a hitpoint**, and passive regeneration is
//!   sixty hitpoints an hour against four hundred or more of damage. Food is not
//!   an optimisation here, it is the constraint — see [`Sustain`].
//!
//! One caveat that is easy to miss: `attempt_pick_pocket` opens with
//! `if (map_members = ^false)`, and `map_members` is the **world** flag
//! `Environment.node.members`, not a per-zone one. Mercantile's default is
//! `members: true`, so this works in Lumbridge; on a free world none of it does.

/// Ticks per second of game time.
pub const TICK_SECONDS: f64 = 0.6;

/// Ticks between passive hitpoint regeneration — `settimer(health_regen, 100)`.
pub const HEALTH_REGEN_TICKS: f64 = 100.0;

/// Hitpoints regenerated per hour with no food at all.
pub const PASSIVE_REGEN_PER_HOUR: f64 = 3_600.0 / (HEALTH_REGEN_TICKS * TICK_SECONDS);

/// The engine's skill roll, from `ScriptOpcode.STAT_RANDOM`:
///
/// ```text
/// value = floor(low * (99 - level) / 98) + floor(high * (level - 1) / 98) + 1
/// success = value > random(256)
/// ```
///
/// So `low` is the chance at level 1 and `high` the chance at level 99, both out
/// of 256, interpolated linearly in between. A `value` of 256 or more never
/// fails, which is why cooking stops burning shrimp at 34 rather than 99.
pub fn stat_random(level: u32, low: i64, high: i64) -> f64 {
    let level = level.clamp(1, 99) as i64;
    let value = (low * (99 - level)) / 98 + (high * (level - 1)) / 98 + 1;
    (value.clamp(0, 256) as f64) / 256.0
}

/// One row of the pickpocket table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pickpocket {
    /// The name used in `mercbot` output.
    pub name: &'static str,
    /// A representative NPC id from the row, for the game bot to search for.
    pub npc: &'static str,
    /// Thieving level required.
    pub level: u32,
    /// Thieving XP, in tenths as the engine stores it.
    pub experience: u32,
    /// Ticks of stun on a failure.
    pub stun_ticks: u32,
    /// Hitpoints lost on a failure.
    pub stun_damage: u32,
    /// `success_chance` low and high, fed to [`stat_random`].
    pub chance: (i64, i64),
    /// Coins a success pays. Every row below `rogue` has a single guaranteed
    /// coin drop, so this is exact rather than an average.
    pub coins: f64,
}

impl Pickpocket {
    /// Chance a single attempt succeeds at this Thieving level.
    pub fn success(&self, level: u32) -> f64 {
        stat_random(level, self.chance.0, self.chance.1)
    }

    /// Thieving XP per successful attempt.
    pub fn xp(&self) -> f64 {
        self.experience as f64 / 10.0
    }

    /// Seconds one attempt costs on average, stun included.
    ///
    /// A success only pays the `p_delay(0)` in `~pick_pocket` plus the tick the
    /// bot needs to re-issue the option, so two ticks. A failure additionally
    /// eats `stun_ticks` before `%action_delay` lets another attempt through.
    pub fn seconds_per_attempt(&self, level: u32) -> f64 {
        let p = self.success(level);
        let ticks = p * 2.0 + (1.0 - p) * (1.0 + self.stun_ticks as f64);
        ticks * TICK_SECONDS
    }

    /// Coins per hour of uninterrupted thieving — before any healing downtime.
    pub fn gp_per_hour(&self, level: u32) -> f64 {
        3_600.0 / self.seconds_per_attempt(level) * self.success(level) * self.coins
    }

    /// Thieving XP per hour of uninterrupted thieving.
    pub fn xp_per_hour(&self, level: u32) -> f64 {
        3_600.0 / self.seconds_per_attempt(level) * self.success(level) * self.xp()
    }

    /// Hitpoints lost per hour of uninterrupted thieving.
    pub fn damage_per_hour(&self, level: u32) -> f64 {
        3_600.0 / self.seconds_per_attempt(level)
            * (1.0 - self.success(level))
            * self.stun_damage as f64
    }

    /// Hitpoints of damage taken per 1000 GP earned.
    ///
    /// The number that actually ranks targets once food is scarce: a farmer pays
    /// three times what a man does for the same hitpoint.
    pub fn hp_per_1000_gp(&self, level: u32) -> f64 {
        let gp = self.gp_per_hour(level);
        if gp <= 0.0 {
            return f64::INFINITY;
        }
        self.damage_per_hour(level) / gp * 1_000.0
    }
}

/// The pickpocket table, cheapest target first.
///
/// Transcribed from `pickpocket.dbrow`. Rows whose loot is a mixed table
/// (`rogue`, `gnome`, `hero`) carry the expected coin value of that table rather
/// than a guaranteed drop, so treat those as averages.
pub const PICKPOCKETS: &[Pickpocket] = &[
    Pickpocket {
        name: "man/woman",
        npc: "man",
        level: 1,
        experience: 80,
        stun_ticks: 8,
        stun_damage: 1,
        chance: (180, 240),
        coins: 3.0,
    },
    Pickpocket {
        name: "farmer",
        npc: "farmer1",
        level: 10,
        experience: 145,
        stun_ticks: 8,
        stun_damage: 1,
        chance: (150, 240),
        coins: 9.0,
    },
    Pickpocket {
        name: "warrior",
        npc: "warrior_woman",
        level: 25,
        experience: 260,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (100, 240),
        coins: 18.0,
    },
    Pickpocket {
        name: "rogue",
        npc: "rogue",
        level: 32,
        experience: 365,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (74, 240),
        // 108/128 of a 25-40 coin roll; the rune and dagger drops are ignored.
        coins: 32.5 * 108.0 / 128.0,
    },
    Pickpocket {
        name: "guard",
        npc: "guard1",
        level: 40,
        experience: 468,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (50, 240),
        coins: 30.0,
    },
    Pickpocket {
        name: "fremennik",
        npc: "viking_man",
        level: 45,
        experience: 650,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (40, 240),
        coins: 40.0,
    },
    Pickpocket {
        name: "knight",
        npc: "knight_of_ardougne",
        level: 55,
        experience: 843,
        stun_ticks: 8,
        stun_damage: 3,
        chance: (50, 240),
        coins: 50.0,
    },
];

/// The best target for a Thieving level, ignoring whether the bot can reach it.
pub fn best_target(level: u32) -> &'static Pickpocket {
    PICKPOCKETS
        .iter()
        .filter(|p| p.level <= level)
        .max_by(|a, b| {
            a.gp_per_hour(level)
                .partial_cmp(&b.gp_per_hour(level))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(&PICKPOCKETS[0])
}

/// A food the bot can produce or buy, and what it heals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Food {
    pub item: &'static str,
    /// Hitpoints restored, from `consume_normal.dbrow`.
    pub heal: u32,
    /// Fishing level to catch it, if the bot makes its own.
    pub fishing_level: u32,
    /// `success_chance` for catching it.
    pub catch_chance: (i64, i64),
    /// Ticks between catch rolls at the spot.
    pub catch_ticks: f64,
    /// `success_chance` for cooking it, from `cooking_generic.dbrow`.
    pub cook_chance: (i64, i64),
}

impl Food {
    /// Raw fish caught per hour at a spot.
    pub fn caught_per_hour(&self, fishing: u32) -> f64 {
        3_600.0 / (self.catch_ticks * TICK_SECONDS)
            * stat_random(fishing, self.catch_chance.0, self.catch_chance.1)
    }

    /// Fraction of raw fish that survives the fire.
    pub fn cook_success(&self, cooking: u32) -> f64 {
        stat_random(cooking, self.cook_chance.0, self.cook_chance.1)
    }

    /// Hitpoints of healing produced per hour spent fishing and cooking.
    ///
    /// Cooking is folded in as a throughput tax rather than timed separately:
    /// what matters is that half the catch is lost at Cooking 1 and none of it
    /// at 34, which is a bigger effect than the seconds spent at the fire.
    pub fn healing_per_hour(&self, fishing: u32, cooking: u32) -> f64 {
        self.caught_per_hour(fishing) * self.cook_success(cooking) * self.heal as f64
    }
}

/// Shrimp: the free option, and the only one the tutorial kit can catch.
pub const SHRIMP: Food = Food {
    item: "shrimp",
    heal: 3,
    fishing_level: 1,
    catch_chance: (48, 256),
    catch_ticks: 5.0,
    cook_chance: (128, 512),
};

/// Trout: needs Fishing 20, a fly rod and feathers, but heals more than twice
/// as much per inventory slot and the river spot is far closer to Lumbridge.
pub const TROUT: Food = Food {
    item: "trout",
    heal: 7,
    fishing_level: 20,
    catch_chance: (32, 192),
    catch_ticks: 5.0,
    cook_chance: (64, 448),
};

/// What a thieving session actually nets once the food is accounted for.
#[derive(Debug, Clone, Copy)]
pub struct Sustain {
    /// Hitpoints an hour of thieving costs.
    pub damage_per_hour: f64,
    /// Hitpoints an hour of thieving gets back for free.
    pub regen_per_hour: f64,
    /// Hitpoints of food an hour of thieving must be paid for with.
    pub deficit_per_hour: f64,
    /// Hitpoints of food an hour of foraging produces.
    pub healing_per_hour: f64,
    /// Fraction of wall-clock time that can be spent thieving.
    pub uptime: f64,
    /// GP per hour of thieving with no interruption.
    pub gross_gp_per_hour: f64,
    /// GP per hour of wall clock, foraging included.
    pub net_gp_per_hour: f64,
}

/// Solve the thieve/forage split for a target and a set of levels.
///
/// The account cannot thieve and fish at once, so the two rates fix the split:
/// an hour of thieving needs `deficit` hitpoints of food, and an hour of
/// foraging makes `healing` of them. Walking between the two is charged as a
/// fixed overhead because the circuit is fixed — Lumbridge to the swamp and
/// back, with the tree on the straight line between them.
pub fn sustain(
    target: &Pickpocket,
    thieving: u32,
    food: &Food,
    fishing: u32,
    cooking: u32,
    travel_seconds_per_trip: f64,
    heals_per_trip: f64,
) -> Sustain {
    let damage_per_hour = target.damage_per_hour(thieving);
    let regen_per_hour = PASSIVE_REGEN_PER_HOUR;
    let deficit_per_hour = (damage_per_hour - regen_per_hour).max(0.0);
    let healing_per_hour = food.healing_per_hour(fishing, cooking);
    let gross = target.gp_per_hour(thieving);

    if deficit_per_hour <= 0.0 {
        return Sustain {
            damage_per_hour,
            regen_per_hour,
            deficit_per_hour,
            healing_per_hour,
            uptime: 1.0,
            gross_gp_per_hour: gross,
            net_gp_per_hour: gross,
        };
    }
    if healing_per_hour <= 0.0 {
        return Sustain {
            damage_per_hour,
            regen_per_hour,
            deficit_per_hour,
            healing_per_hour,
            uptime: 0.0,
            gross_gp_per_hour: gross,
            net_gp_per_hour: 0.0,
        };
    }

    // Per hour of thieving: the foraging that funds it, plus the walk. A trip
    // carries `heals_per_trip` hitpoints home, so the number of round trips an
    // hour of thieving needs is deficit / heals_per_trip.
    let forage_hours = deficit_per_hour / healing_per_hour;
    let trips = deficit_per_hour / heals_per_trip.max(1.0);
    let travel_hours = trips * travel_seconds_per_trip / 3_600.0;
    let uptime = 1.0 / (1.0 + forage_hours + travel_hours);

    Sustain {
        damage_per_hour,
        regen_per_hour,
        deficit_per_hour,
        healing_per_hour,
        uptime,
        gross_gp_per_hour: gross,
        net_gp_per_hour: gross * uptime,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn man() -> &'static Pickpocket {
        &PICKPOCKETS[0]
    }

    #[test]
    fn the_roll_matches_the_engine_formula() {
        // Hand-evaluated from ScriptOpcode.STAT_RANDOM at the ends of the range.
        // Level 1 collapses to `low`, level 99 to `high`, both plus one.
        assert_eq!(stat_random(1, 180, 240), 181.0 / 256.0);
        assert_eq!(stat_random(99, 180, 240), 241.0 / 256.0);
    }

    #[test]
    fn a_roll_of_256_never_fails() {
        // Cooking shrimp is 128..512, and 512 * 33 / 98 + 128 * 65 / 98 + 1 is
        // 257 — so shrimp stop burning at Cooking 34, not at 99.
        assert!(SHRIMP.cook_success(33) < 1.0);
        assert_eq!(SHRIMP.cook_success(34), 1.0);
    }

    #[test]
    fn a_man_pays_three_coins_seven_times_in_ten() {
        let p = man().success(1);
        assert!((p - 0.707).abs() < 0.001, "{p}");
        let gp = man().gp_per_hour(1);
        assert!((3_100.0..3_200.0).contains(&gp), "{gp}");
    }

    #[test]
    fn a_farmer_is_worth_more_per_hitpoint_than_a_man() {
        // The whole reason to spend seven minutes reaching Thieving 10: the
        // farmer pays three times as much for the same one-hitpoint stun.
        let farmer = &PICKPOCKETS[1];
        assert!(farmer.gp_per_hour(10) > 2.0 * man().gp_per_hour(10));
        // 68 hitpoints per 1000 GP against 125 — a little under half.
        assert!(farmer.hp_per_1000_gp(10) < man().hp_per_1000_gp(10) / 1.8);
    }

    #[test]
    fn best_target_respects_the_level_gate() {
        assert_eq!(best_target(1).name, "man/woman");
        assert_eq!(best_target(9).name, "man/woman");
        assert_eq!(best_target(10).name, "farmer");
        assert_eq!(best_target(99).name, "knight");
    }

    #[test]
    fn passive_regeneration_cannot_keep_up() {
        // Sixty hitpoints an hour against four hundred of damage. This is the
        // fact that forces a food loop rather than an idle-and-heal loop.
        assert_eq!(PASSIVE_REGEN_PER_HOUR, 60.0);
        assert!(man().damage_per_hour(1) > 6.0 * PASSIVE_REGEN_PER_HOUR);
    }

    #[test]
    fn the_food_loop_costs_more_than_half_the_clock_at_base_levels() {
        let s = sustain(&PICKPOCKETS[1], 10, &SHRIMP, 1, 1, 138.0, 81.0);
        assert!(s.uptime < 0.5, "uptime {}", s.uptime);
        assert!(s.net_gp_per_hour < s.gross_gp_per_hour);
    }

    #[test]
    fn levelling_fishing_and_cooking_buys_back_the_clock() {
        let base = sustain(&PICKPOCKETS[1], 10, &SHRIMP, 1, 1, 138.0, 81.0);
        let trained = sustain(&PICKPOCKETS[1], 10, &SHRIMP, 30, 34, 138.0, 81.0);
        assert!(trained.uptime > base.uptime * 1.5);
    }

    #[test]
    fn trout_heals_more_per_catch_than_shrimp() {
        // Same roll cadence, better fish: the reason tier two moves to the river.
        assert!(TROUT.healing_per_hour(20, 34) > SHRIMP.healing_per_hour(20, 34));
    }

    #[test]
    fn no_deficit_means_full_uptime() {
        // A hypothetical target that never fails would need no food at all.
        let harmless = Pickpocket {
            stun_damage: 0,
            ..*man()
        };
        let s = sustain(&harmless, 1, &SHRIMP, 1, 1, 138.0, 81.0);
        assert_eq!(s.uptime, 1.0);
        assert_eq!(s.net_gp_per_hour, s.gross_gp_per_hour);
    }
}

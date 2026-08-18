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
    /// The row's `loot` entries, in the order the dbrow lists them.
    ///
    /// Transcribed, not summarised: the probabilities are *derived* from these
    /// by [`Pickpocket::drop_chances`], because reading them off by eye gets
    /// them wrong. See that function for why.
    pub loot: &'static [Drop],
}

/// One `data=loot,<item>,<min>,<max>,<numerator>` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drop {
    pub item: &'static str,
    pub min: u32,
    pub max: u32,
    /// Weight out of the *current* denominator, which shrinks as the loop runs.
    pub numerator: i64,
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

    /// Every drop this row can produce, with the chance the engine gives it.
    ///
    /// `pick_pocket_check_for_reward` is not a pick-one table. It walks the
    /// entries **backwards** with a denominator that starts at 128 and shrinks
    /// by each numerator as it goes, rolling every entry:
    ///
    /// ```text
    /// $roll = random($denominator)          // before the subtraction
    /// $denominator = $denominator - $numerator
    /// if ($roll >= $denominator) { inv_add(...) }
    /// ```
    ///
    /// Two things fall out of that, and both are easy to get wrong by eye:
    ///
    /// * **A success can grant several items at once.** There is no `return`
    ///   after a hit, unlike the stall table's `stealing_check_for_reward`. A
    ///   rogue can pay coins *and* air runes *and* wine in one pick.
    /// * **The last entry processed — the dbrow's first — is usually
    ///   guaranteed**, because the numerators sum to 128 and drive the
    ///   denominator to zero. So the rogue's coins are certain, not 108/128.
    ///
    /// The exception is a row whose numerators sum to *less* than 128: the
    /// farmer's single entry is 123, so 5 of 128 successful picks pay nothing.
    pub fn drop_chances(&self) -> Vec<(Drop, f64)> {
        let mut denominator: i64 = 128;
        let mut out = Vec::with_capacity(self.loot.len());
        for drop in self.loot.iter().rev() {
            let before = denominator;
            let after = denominator - drop.numerator;
            // `random(n)` yields 0..n-1, so a hit needs `roll >= after`. A
            // non-positive `after` cannot be missed; `before <= 0` means the
            // denominator has already been exhausted and the test is trivially
            // true (the watchman's second entry does exactly this).
            let chance = if before <= 0 || after <= 0 {
                1.0
            } else {
                (before - after) as f64 / before as f64
            };
            out.push((*drop, chance));
            denominator = after;
        }
        out.reverse();
        out
    }

    /// Expected coins from one success, across the whole loot table.
    pub fn coins(&self) -> f64 {
        self.drop_chances()
            .iter()
            .filter(|(drop, _)| drop.item == "coins")
            .map(|(drop, chance)| (drop.min + drop.max) as f64 / 2.0 * chance)
            .sum()
    }

    /// Coins per hour of uninterrupted thieving — before any healing downtime.
    ///
    /// Coins only. The rogue's runes and the gnome's worms are real income but
    /// they have to be carried and sold, so they do not belong in a rate that
    /// the sustain model treats as spendable.
    pub fn gp_per_hour(&self, level: u32) -> f64 {
        3_600.0 / self.seconds_per_attempt(level) * self.success(level) * self.coins()
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
/// Every field is transcribed from `pickpocket.dbrow` exactly as the game
/// stores it, including the loot numerators. Nothing here is a summary: the
/// probabilities come from replaying the engine's own reward loop in
/// [`Pickpocket::drop_chances`], which is the only way to get them right.
pub const PICKPOCKETS: &[Pickpocket] = &[
    Pickpocket {
        name: "man/woman",
        npc: "man",
        level: 1,
        experience: 80,
        stun_ticks: 8,
        stun_damage: 1,
        chance: (180, 240),
        loot: &[Drop {
            item: "coins",
            min: 3,
            max: 3,
            numerator: 128,
        }],
    },
    Pickpocket {
        name: "farmer",
        npc: "farmer1",
        level: 10,
        experience: 145,
        stun_ticks: 8,
        stun_damage: 1,
        chance: (150, 240),
        loot: &[Drop {
            item: "coins",
            min: 9,
            max: 9,
            numerator: 123,
        }],
    },
    Pickpocket {
        name: "warrior",
        npc: "warrior_woman",
        level: 25,
        experience: 260,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (100, 240),
        loot: &[Drop {
            item: "coins",
            min: 18,
            max: 18,
            numerator: 128,
        }],
    },
    Pickpocket {
        name: "rogue",
        npc: "rogue",
        level: 32,
        experience: 365,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (74, 240),
        loot: &[
            Drop {
                item: "coins",
                min: 25,
                max: 40,
                numerator: 108,
            },
            Drop {
                item: "airrune",
                min: 8,
                max: 8,
                numerator: 8,
            },
            Drop {
                item: "jug_wine",
                min: 1,
                max: 1,
                numerator: 6,
            },
            Drop {
                item: "lockpick",
                min: 1,
                max: 1,
                numerator: 5,
            },
            Drop {
                item: "iron_dagger_p",
                min: 1,
                max: 1,
                numerator: 1,
            },
        ],
    },
    Pickpocket {
        name: "guard",
        npc: "guard1",
        level: 40,
        experience: 468,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (50, 240),
        loot: &[Drop {
            item: "coins",
            min: 30,
            max: 30,
            numerator: 128,
        }],
    },
    Pickpocket {
        name: "fremennik",
        npc: "viking_man",
        level: 45,
        experience: 650,
        stun_ticks: 8,
        stun_damage: 2,
        chance: (40, 240),
        loot: &[Drop {
            item: "coins",
            min: 40,
            max: 40,
            numerator: 128,
        }],
    },
    Pickpocket {
        name: "knight",
        npc: "knight_of_ardougne",
        level: 55,
        experience: 843,
        stun_ticks: 8,
        stun_damage: 3,
        chance: (50, 240),
        loot: &[Drop {
            item: "coins",
            min: 50,
            max: 50,
            numerator: 128,
        }],
    },
    Pickpocket {
        name: "watchman",
        npc: "yanille_watchman",
        level: 65,
        experience: 1375,
        stun_ticks: 8,
        stun_damage: 3,
        chance: (15, 160),
        loot: &[
            Drop {
                item: "coins",
                min: 60,
                max: 60,
                numerator: 128,
            },
            Drop {
                item: "bread",
                min: 1,
                max: 1,
                numerator: 128,
            },
        ],
    },
    Pickpocket {
        name: "paladin",
        npc: "paladin",
        level: 70,
        experience: 1518,
        stun_ticks: 8,
        stun_damage: 3,
        chance: (50, 150),
        loot: &[
            Drop {
                item: "coins",
                min: 80,
                max: 80,
                numerator: 128,
            },
            Drop {
                item: "chaosrune",
                min: 2,
                max: 2,
                numerator: 128,
            },
        ],
    },
    Pickpocket {
        name: "gnome",
        npc: "gnome",
        level: 75,
        experience: 1985,
        stun_ticks: 8,
        stun_damage: 1,
        chance: (50, 240),
        loot: &[
            Drop {
                item: "king_worm",
                min: 1,
                max: 1,
                numerator: 55,
            },
            Drop {
                item: "coins",
                min: 300,
                max: 300,
                numerator: 30,
            },
            Drop {
                item: "swamp_toad",
                min: 1,
                max: 1,
                numerator: 28,
            },
            Drop {
                item: "gold_ore",
                min: 1,
                max: 1,
                numerator: 8,
            },
            Drop {
                item: "earthrune",
                min: 1,
                max: 1,
                numerator: 5,
            },
            Drop {
                item: "fire_orb",
                min: 1,
                max: 1,
                numerator: 2,
            },
        ],
    },
    Pickpocket {
        name: "hero",
        npc: "hero",
        level: 80,
        experience: 2751,
        stun_ticks: 8,
        stun_damage: 3,
        chance: (20, 120),
        loot: &[
            Drop {
                item: "coins",
                min: 200,
                max: 300,
                numerator: 105,
            },
            Drop {
                item: "deathrune",
                min: 2,
                max: 2,
                numerator: 8,
            },
            Drop {
                item: "jug_wine",
                min: 1,
                max: 1,
                numerator: 6,
            },
            Drop {
                item: "bloodrune",
                min: 1,
                max: 1,
                numerator: 5,
            },
            Drop {
                item: "fire_orb",
                min: 1,
                max: 1,
                numerator: 2,
            },
            Drop {
                item: "diamond",
                min: 1,
                max: 1,
                numerator: 1,
            },
            Drop {
                item: "gold_ore",
                min: 1,
                max: 1,
                numerator: 1,
            },
        ],
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
    fn the_reward_loop_guarantees_the_first_entry_when_the_weights_sum_to_128() {
        // The rogue's coins have numerator 108 out of 128, which reads like an
        // 84% chance and is not. The loop processes entries backwards, and by
        // the time it reaches the coins the denominator has been shaved to 108,
        // so `random(108) >= 0` always holds. Reading it off by eye priced the
        // rogue 16% low.
        let rogue = PICKPOCKETS.iter().find(|p| p.name == "rogue").unwrap();
        let chances = rogue.drop_chances();
        let coins = chances.iter().find(|(d, _)| d.item == "coins").unwrap();
        assert_eq!(coins.1, 1.0);
        assert_eq!(rogue.coins(), 32.5);
    }

    #[test]
    fn weights_that_sum_below_128_leave_a_dud_chance() {
        // The farmer's single entry is 123, not 128, so 5 successful picks in
        // every 128 pay nothing at all.
        let farmer = PICKPOCKETS.iter().find(|p| p.name == "farmer").unwrap();
        let (_, chance) = farmer.drop_chances()[0];
        assert!((chance - 123.0 / 128.0).abs() < 1e-9, "{chance}");
        assert!((farmer.coins() - 8.648).abs() < 0.001, "{}", farmer.coins());
    }

    #[test]
    fn a_success_can_grant_several_items_at_once() {
        // There is no `return` after a hit — unlike the stall table. Every entry
        // is rolled, so a rogue can pay coins and runes and wine in one pick.
        let rogue = PICKPOCKETS.iter().find(|p| p.name == "rogue").unwrap();
        let chances = rogue.drop_chances();
        assert_eq!(chances.len(), 5);
        assert!(chances.iter().all(|(_, c)| *c > 0.0));
        // Everything besides the guaranteed coins is a genuine long shot.
        let extras: Vec<f64> = chances
            .iter()
            .filter(|(d, _)| d.item != "coins")
            .map(|(_, c)| *c)
            .collect();
        assert!(extras.iter().all(|c| *c < 0.08), "{extras:?}");
    }

    #[test]
    fn an_exhausted_denominator_still_pays() {
        // The watchman has two entries of 128 each. The first drives the
        // denominator to zero, and the second then tests `random(0) >= -128`,
        // which the engine treats as a hit — so bread and coins both land.
        let watchman = PICKPOCKETS.iter().find(|p| p.name == "watchman").unwrap();
        assert!(watchman.drop_chances().iter().all(|(_, c)| *c == 1.0));
        assert_eq!(watchman.coins(), 60.0);
    }

    #[test]
    fn the_gnome_pays_worms_far_more_often_than_coins() {
        // A row where the guaranteed entry is worthless: king worms every time,
        // 300 coins only about a third of the time.
        let gnome = PICKPOCKETS.iter().find(|p| p.name == "gnome").unwrap();
        let chances = gnome.drop_chances();
        let worm = chances.iter().find(|(d, _)| d.item == "king_worm").unwrap();
        let coins = chances.iter().find(|(d, _)| d.item == "coins").unwrap();
        assert_eq!(worm.1, 1.0);
        assert!(coins.1 < 0.4, "{}", coins.1);
    }

    #[test]
    fn every_row_pays_something_on_a_success() {
        for row in PICKPOCKETS {
            let total: f64 = row.drop_chances().iter().map(|(_, c)| c).sum();
            assert!(total > 0.9, "{} pays nothing: {total}", row.name);
        }
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
        // 70 hitpoints per 1000 GP against 125 — a little under half. The
        // farmer's 9 coins land on only 123 successes in 128, so this is 8.65
        // per pick, not 9.
        assert!((farmer.coins() - 8.648).abs() < 0.001);
        assert!(farmer.hp_per_1000_gp(10) < man().hp_per_1000_gp(10) / 1.7);
    }

    #[test]
    fn best_target_respects_the_level_gate() {
        assert_eq!(best_target(1).name, "man/woman");
        assert_eq!(best_target(9).name, "man/woman");
        assert_eq!(best_target(10).name, "farmer");
        // The gnome, not the knight: 300 coins a third of the time plus a
        // guaranteed king worm, and only one hitpoint of stun per failure.
        assert_eq!(best_target(99).name, "gnome");
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

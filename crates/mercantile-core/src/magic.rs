//! Magic training routes.
//!
//! Getting an account to level 55 (High Level Alchemy) and then 66 (the Wizards'
//! Guild, and with it an unbounded nature rune supply) is the gate on the whole
//! alchemy business, so it is worth planning rather than guessing.
//!
//! Every spell below is taken from
//! `server/content/scripts/skill_combat/configs/magic/magic_combat_spells.dbrow`
//! and `skill_magic/configs/magic_spells.dbrow`; rune values are the `cost`
//! fields in `skill_magic/configs/runes.obj`. The two facts that make the
//! planning interesting:
//!
//! * **Every combat spell takes the same five ticks**, so "fastest" is simply
//!   "most XP per cast" — there is no speed/efficiency axis within combat spells.
//!   Only alchemy breaks that: low alchemy is a three-tick cast.
//! * **XP is paid whether or not the spell hits**, so a splashed spell trains at
//!   its full base rate. That is what makes the debuff spells usable at all —
//!   see the `splash` module in the game bot.

use std::collections::BTreeMap;

/// What a spell needs from its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRequirement {
    /// Any attackable NPC.
    Any,
    /// Skeletons, zombies, ghosts and shades only.
    Undead,
    /// An item in the inventory, which the cast destroys.
    InventoryItem,
}

/// How a spell behaves when cast repeatedly on one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeatability {
    /// Damage spells: the target dies eventually unless every cast splashes.
    Damage,
    /// Debuff spells: a *landed* cast blocks the next one until the NPC's stats
    /// restore, so these are only repeatable while every cast misses.
    RequiresSplash,
    /// Alchemy: no target NPC at all.
    Item,
}

/// A trainable spell.
#[derive(Debug, Clone, Copy)]
pub struct Spell {
    pub name: &'static str,
    /// Magic interface component id, for the bot SDK.
    pub component: u16,
    pub level: u32,
    pub xp: f64,
    /// Game ticks the cast occupies.
    pub ticks: u32,
    pub runes: &'static [(&'static str, u32)],
    pub target: TargetRequirement,
    pub repeatability: Repeatability,
}

impl Spell {
    /// Seconds per cast on a normal-rate world.
    pub fn seconds(&self) -> f64 {
        self.ticks as f64 * 0.6
    }

    /// XP per hour of uninterrupted casting.
    pub fn xp_per_hour(&self) -> f64 {
        self.xp * 3_600.0 / self.seconds()
    }

    /// GP of runes per cast at the given prices.
    pub fn rune_cost(&self, prices: &RunePrices) -> f64 {
        self.runes
            .iter()
            .map(|(rune, count)| prices.price(rune) * *count as f64)
            .sum()
    }

    /// GP per XP at the given prices — the efficiency measure.
    pub fn gp_per_xp(&self, prices: &RunePrices) -> f64 {
        self.rune_cost(prices) / self.xp
    }

    /// Whether the runes are all sold by Aubury and Betty, who have no entry
    /// requirements. Nature, law, cosmic, blood and soul runes are not.
    pub fn shop_suppliable(&self) -> bool {
        const SHOP_RUNES: [&str; 8] = [
            "airrune",
            "waterrune",
            "earthrune",
            "firerune",
            "mindrune",
            "bodyrune",
            "chaosrune",
            "deathrune",
        ];
        self.runes.iter().all(|(rune, _)| SHOP_RUNES.contains(rune))
    }
}

/// Every spell worth training on, richest XP first within each tier.
pub const SPELLS: &[Spell] = &[
    Spell {
        name: "wind strike",
        component: 1152,
        level: 1,
        xp: 5.5,
        ticks: 5,
        runes: &[("airrune", 1), ("mindrune", 1)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "confuse",
        component: 1153,
        level: 3,
        xp: 13.0,
        ticks: 5,
        runes: &[("bodyrune", 1), ("waterrune", 3), ("earthrune", 2)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::RequiresSplash,
    },
    Spell {
        name: "water strike",
        component: 1154,
        level: 5,
        xp: 7.5,
        ticks: 5,
        runes: &[("mindrune", 1), ("waterrune", 1), ("airrune", 1)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "earth strike",
        component: 1156,
        level: 9,
        xp: 9.5,
        ticks: 5,
        runes: &[("mindrune", 1), ("earthrune", 2), ("airrune", 1)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "weaken",
        component: 1157,
        level: 11,
        xp: 21.0,
        ticks: 5,
        runes: &[("bodyrune", 1), ("waterrune", 3), ("earthrune", 2)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::RequiresSplash,
    },
    Spell {
        name: "fire strike",
        component: 1158,
        level: 13,
        xp: 11.5,
        ticks: 5,
        runes: &[("mindrune", 1), ("firerune", 3), ("airrune", 2)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "wind bolt",
        component: 1160,
        level: 17,
        xp: 13.5,
        ticks: 5,
        runes: &[("chaosrune", 1), ("airrune", 2)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "curse",
        component: 1161,
        level: 19,
        xp: 29.0,
        ticks: 5,
        runes: &[("bodyrune", 1), ("waterrune", 2), ("earthrune", 3)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::RequiresSplash,
    },
    Spell {
        name: "low alchemy",
        component: 1162,
        level: 21,
        xp: 31.0,
        ticks: 3,
        runes: &[("naturerune", 1), ("firerune", 3)],
        target: TargetRequirement::InventoryItem,
        repeatability: Repeatability::Item,
    },
    Spell {
        name: "water bolt",
        component: 1163,
        level: 23,
        xp: 16.5,
        ticks: 5,
        runes: &[("chaosrune", 1), ("waterrune", 2), ("airrune", 2)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "earth bolt",
        component: 1166,
        level: 29,
        xp: 19.5,
        ticks: 5,
        runes: &[("chaosrune", 1), ("earthrune", 3), ("airrune", 2)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "fire bolt",
        component: 1169,
        level: 35,
        xp: 22.5,
        ticks: 5,
        runes: &[("chaosrune", 1), ("firerune", 4), ("airrune", 3)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "crumble undead",
        component: 1171,
        level: 39,
        xp: 49.0,
        ticks: 5,
        runes: &[("chaosrune", 1), ("airrune", 2), ("earthrune", 2)],
        target: TargetRequirement::Undead,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "wind blast",
        component: 1172,
        level: 41,
        xp: 25.5,
        ticks: 5,
        runes: &[("deathrune", 1), ("airrune", 3)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "water blast",
        component: 1175,
        level: 47,
        xp: 28.5,
        ticks: 5,
        runes: &[("deathrune", 1), ("waterrune", 3), ("airrune", 3)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "earth blast",
        component: 1177,
        level: 53,
        xp: 31.5,
        ticks: 5,
        runes: &[("deathrune", 1), ("earthrune", 4), ("airrune", 3)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
    Spell {
        name: "high alchemy",
        component: 1178,
        level: 55,
        xp: 65.0,
        ticks: 5,
        runes: &[("naturerune", 1), ("firerune", 5)],
        target: TargetRequirement::InventoryItem,
        repeatability: Repeatability::Item,
    },
    Spell {
        name: "fire blast",
        component: 1181,
        level: 59,
        xp: 34.5,
        ticks: 5,
        runes: &[("deathrune", 1), ("firerune", 5), ("airrune", 4)],
        target: TargetRequirement::Any,
        repeatability: Repeatability::Damage,
    },
];

/// GP per rune. Defaults are the game's own `cost` values, which shop prices
/// track closely; override the ones you actually pay.
#[derive(Debug, Clone)]
pub struct RunePrices {
    prices: BTreeMap<String, f64>,
}

impl Default for RunePrices {
    fn default() -> Self {
        let mut prices = BTreeMap::new();
        for (rune, cost) in [
            ("airrune", 4.0),
            ("waterrune", 4.0),
            ("earthrune", 4.0),
            ("firerune", 4.0),
            ("mindrune", 3.0),
            ("bodyrune", 3.0),
            ("chaosrune", 15.0),
            ("deathrune", 30.0),
            ("naturerune", 20.0),
            ("lawrune", 40.0),
            ("cosmicrune", 15.0),
            ("bloodrune", 50.0),
            ("soulrune", 1250.0),
        ] {
            prices.insert(rune.to_string(), cost);
        }
        Self { prices }
    }
}

impl RunePrices {
    pub fn price(&self, rune: &str) -> f64 {
        self.prices.get(rune).copied().unwrap_or(0.0)
    }

    /// Override one rune's price — e.g. nature runes from their live pool.
    pub fn set(&mut self, rune: &str, price: f64) {
        self.prices.insert(rune.to_string(), price);
    }

    /// A staff of fire supplies fire runes for nothing, permanently.
    pub fn with_fire_staff(mut self) -> Self {
        self.prices.insert("firerune".to_string(), 0.0);
        self
    }
}

/// What to optimise for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    /// Least GP spent per XP.
    Cheapest,
    /// Most XP per hour.
    Fastest,
}

/// What the planner is allowed to use.
#[derive(Debug, Clone, Copy)]
pub struct Constraints {
    /// Only spells whose runes Aubury and Betty stock — i.e. unbounded supply.
    pub shop_runes_only: bool,
    /// Whether an undead target (Varrock sewers, Draynor Manor) is available.
    pub allow_undead: bool,
    /// Whether alchemy may be used, which needs both nature runes and a stream
    /// of items to destroy — both supply-limited until the Wizards' Guild opens.
    pub allow_alchemy: bool,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            shop_runes_only: true,
            allow_undead: true,
            allow_alchemy: false,
        }
    }
}

/// One stretch of the plan cast with a single spell.
#[derive(Debug, Clone)]
pub struct Leg {
    pub spell: Spell,
    pub from_level: u32,
    pub to_level: u32,
    pub casts: u64,
    pub seconds: f64,
    pub rune_gp: f64,
}

/// A full training plan.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub legs: Vec<Leg>,
}

impl Plan {
    pub fn casts(&self) -> u64 {
        self.legs.iter().map(|leg| leg.casts).sum()
    }

    pub fn seconds(&self) -> f64 {
        self.legs.iter().map(|leg| leg.seconds).sum()
    }

    pub fn hours(&self) -> f64 {
        self.seconds() / 3_600.0
    }

    /// GP of runes for the whole plan. Alchemy legs are *not* netted against
    /// what the alch pays back — that depends on which items are fed in, and
    /// belongs to `alch-scan`, not here.
    pub fn rune_gp(&self) -> f64 {
        self.legs.iter().map(|leg| leg.rune_gp).sum()
    }
}

/// XP required for a level, on the standard RuneScape curve.
pub fn xp_for_level(level: u32) -> f64 {
    let mut points = 0.0f64;
    for l in 1..level {
        points += (l as f64 + 300.0 * 2f64.powf(l as f64 / 7.0)).floor();
    }
    (points / 4.0).floor()
}

/// The level a given XP total corresponds to.
pub fn level_for_xp(xp: f64) -> u32 {
    (1..=99).rev().find(|l| xp >= xp_for_level(*l)).unwrap_or(1)
}

/// Whether a spell is usable under the constraints.
fn allowed(spell: &Spell, constraints: &Constraints) -> bool {
    if constraints.shop_runes_only && !spell.shop_suppliable() {
        return false;
    }
    match spell.target {
        TargetRequirement::Undead if !constraints.allow_undead => false,
        TargetRequirement::InventoryItem if !constraints.allow_alchemy => false,
        _ => true,
    }
}

/// The best spell available at `level` under the objective and constraints.
pub fn best_spell(
    level: u32,
    objective: Objective,
    prices: &RunePrices,
    constraints: &Constraints,
) -> Option<&'static Spell> {
    SPELLS
        .iter()
        .filter(|spell| spell.level <= level && allowed(spell, constraints))
        .min_by(|a, b| {
            let (x, y) = match objective {
                // Cheapest: least GP per XP. Ties break towards more XP per hour,
                // since two equally cheap spells are not equally quick.
                Objective::Cheapest => (
                    (a.gp_per_xp(prices), -a.xp_per_hour()),
                    (b.gp_per_xp(prices), -b.xp_per_hour()),
                ),
                // Fastest: most XP per hour, ties broken by cost.
                Objective::Fastest => (
                    (-a.xp_per_hour(), a.gp_per_xp(prices)),
                    (-b.xp_per_hour(), b.gp_per_xp(prices)),
                ),
            };
            x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Plan a route from `from_level` to `to_level`.
///
/// Spell choice only ever improves with level, so the optimal plan is: take the
/// best spell available now, cast it until something better unlocks, repeat.
pub fn plan(
    from_level: u32,
    to_level: u32,
    objective: Objective,
    prices: &RunePrices,
    constraints: &Constraints,
) -> Plan {
    let mut plan = Plan::default();
    let mut level = from_level.max(1);
    let mut xp = xp_for_level(level);
    let target_xp = xp_for_level(to_level);

    while xp < target_xp {
        let Some(spell) = best_spell(level, objective, prices, constraints) else {
            break;
        };
        // Cast until either the target or the next spell unlock, whichever first.
        let next_unlock = SPELLS
            .iter()
            .filter(|candidate| {
                candidate.level > level
                    && allowed(candidate, constraints)
                    && is_better(candidate, spell, objective, prices)
            })
            .map(|candidate| candidate.level)
            .min();
        let leg_target_xp = match next_unlock {
            Some(unlock) => target_xp.min(xp_for_level(unlock)),
            None => target_xp,
        };
        let casts = ((leg_target_xp - xp) / spell.xp).ceil().max(0.0) as u64;
        if casts == 0 {
            break;
        }
        let from = level_for_xp(xp);
        xp += casts as f64 * spell.xp;
        level = level_for_xp(xp);
        plan.legs.push(Leg {
            spell: *spell,
            from_level: from,
            to_level: level.min(to_level),
            casts,
            seconds: casts as f64 * spell.seconds(),
            rune_gp: casts as f64 * spell.rune_cost(prices),
        });
    }
    plan
}

fn is_better(a: &Spell, b: &Spell, objective: Objective, prices: &RunePrices) -> bool {
    match objective {
        Objective::Cheapest => a.gp_per_xp(prices) < b.gp_per_xp(prices),
        Objective::Fastest => a.xp_per_hour() > b.xp_per_hour(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_xp_curve_matches_the_game() {
        assert_eq!(xp_for_level(1), 0.0);
        assert_eq!(xp_for_level(2), 83.0);
        assert_eq!(xp_for_level(21), 5_018.0);
        assert_eq!(xp_for_level(43), 50_339.0);
        assert_eq!(xp_for_level(55), 166_636.0);
        assert_eq!(xp_for_level(99), 13_034_431.0);
        assert_eq!(xp_for_level(66), 496_254.0);
        assert_eq!(level_for_xp(5_017.0), 20);
        assert_eq!(level_for_xp(5_018.0), 21);
    }

    #[test]
    fn every_combat_spell_casts_at_the_same_speed() {
        // Which is why "fastest" reduces to "most XP per cast" outside alchemy.
        for spell in SPELLS {
            if spell.repeatability != Repeatability::Item {
                assert_eq!(spell.ticks, 5, "{}", spell.name);
            }
        }
        assert_eq!(
            SPELLS
                .iter()
                .find(|s| s.name == "low alchemy")
                .unwrap()
                .ticks,
            3,
            "low alchemy is the one genuinely faster cast"
        );
    }

    #[test]
    fn crumble_undead_is_the_efficiency_outlier() {
        let prices = RunePrices::default();
        let crumble = SPELLS.iter().find(|s| s.name == "crumble undead").unwrap();
        for spell in SPELLS {
            if spell.level <= 66 && spell.name != "crumble undead" && spell.shop_suppliable() {
                assert!(
                    crumble.gp_per_xp(&prices) <= spell.gp_per_xp(&prices),
                    "{} is cheaper per xp than crumble undead",
                    spell.name
                );
            }
        }
        assert!(crumble.xp_per_hour() > 58_000.0);
    }

    #[test]
    fn shop_supply_is_identified_correctly() {
        let by_name = |name: &str| SPELLS.iter().find(|s| s.name == name).unwrap();
        assert!(by_name("curse").shop_suppliable());
        assert!(
            by_name("crumble undead").shop_suppliable(),
            "chaos runes are stocked"
        );
        assert!(
            by_name("fire blast").shop_suppliable(),
            "death runes are stocked"
        );
        assert!(
            !by_name("high alchemy").shop_suppliable(),
            "nature runes are not"
        );
        assert!(!by_name("low alchemy").shop_suppliable());
    }

    #[test]
    fn a_cheap_plan_to_55_uses_only_shop_runes() {
        let prices = RunePrices::default();
        let plan = plan(1, 55, Objective::Cheapest, &prices, &Constraints::default());
        assert!(!plan.legs.is_empty());
        for leg in &plan.legs {
            assert!(
                leg.spell.shop_suppliable(),
                "{} needs off-shop runes",
                leg.spell.name
            );
        }
        assert_eq!(plan.legs.last().unwrap().to_level, 55);
        // A few hours and a five-figure rune bill, not more.
        assert!(
            (2.0..12.0).contains(&plan.hours()),
            "{} hours",
            plan.hours()
        );
        assert!(plan.rune_gp() < 200_000.0, "{} gp", plan.rune_gp());
    }

    #[test]
    fn the_fast_plan_is_never_slower_than_the_cheap_one() {
        let prices = RunePrices::default();
        let cheap = plan(1, 55, Objective::Cheapest, &prices, &Constraints::default());
        let fast = plan(1, 55, Objective::Fastest, &prices, &Constraints::default());
        assert!(
            fast.hours() <= cheap.hours() + 1e-6,
            "{} vs {}",
            fast.hours(),
            cheap.hours()
        );
        assert!(cheap.rune_gp() <= fast.rune_gp() + 1e-6);
    }

    #[test]
    fn without_an_undead_target_the_plan_still_completes() {
        let prices = RunePrices::default();
        let constraints = Constraints {
            allow_undead: false,
            ..Default::default()
        };
        let plan = plan(1, 55, Objective::Fastest, &prices, &constraints);
        assert_eq!(plan.legs.last().unwrap().to_level, 55);
        for leg in &plan.legs {
            assert_ne!(leg.spell.target, TargetRequirement::Undead);
        }
    }

    #[test]
    fn alchemy_only_appears_when_it_is_allowed() {
        let prices = RunePrices::default();
        let without = plan(21, 55, Objective::Fastest, &prices, &Constraints::default());
        assert!(without
            .legs
            .iter()
            .all(|leg| leg.spell.repeatability != Repeatability::Item));

        let with = plan(
            21,
            55,
            Objective::Fastest,
            &prices,
            &Constraints {
                allow_alchemy: true,
                shop_runes_only: false,
                ..Default::default()
            },
        );
        assert!(
            with.legs.iter().any(|leg| leg.spell.name == "low alchemy"),
            "low alchemy is 62k xp/hour, the fastest thing available below 55"
        );
    }

    #[test]
    fn a_plan_that_is_already_finished_is_empty() {
        let prices = RunePrices::default();
        assert!(plan(
            55,
            55,
            Objective::Cheapest,
            &prices,
            &Constraints::default()
        )
        .legs
        .is_empty());
        assert!(plan(
            60,
            55,
            Objective::Cheapest,
            &prices,
            &Constraints::default()
        )
        .legs
        .is_empty());
    }

    #[test]
    fn a_fire_staff_removes_the_fire_rune_bill() {
        let plain = RunePrices::default();
        let staffed = RunePrices::default().with_fire_staff();
        let high_alch = SPELLS.iter().find(|s| s.name == "high alchemy").unwrap();
        assert_eq!(high_alch.rune_cost(&plain), 20.0 + 5.0 * 4.0);
        assert_eq!(high_alch.rune_cost(&staffed), 20.0);
    }
}

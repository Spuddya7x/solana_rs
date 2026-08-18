//! Alchemy economics.
//!
//! Alchemy is what ties the on-chain price of an item to the game's own economy,
//! and it is the reason a Mercantile pool has a floor at all. The constants and
//! formulas here are taken from the game's own scripts, not from memory:
//!
//! * `server/content/scripts/skill_magic/scripts/spells/alchemy.rs2` —
//!   `max(scale(6, 10, oc_cost($item)), 1)` for high alchemy, `scale(4, 10, ...)`
//!   for low, and `p_delay(3)` after the cast.
//! * `server/content/scripts/skill_magic/configs/magic_spells.dbrow` —
//!   high alchemy needs level 55, one nature rune and five fire runes, and pays
//!   65 Magic XP; low alchemy needs level 21, one nature rune and three fire runes.
//!
//! The arbitrage this enables: a pool's floor is `0.9 x lowalch = 0.36 x cost`,
//! while high alchemy pays `0.6 x cost`. An item bought at its floor and high
//! alched returns about **1.67x** its purchase price in GP, less the nature rune.

/// High alchemy pays this fraction of an item's shop cost.
pub const HIGH_ALCH_MULT: f64 = 0.6;
/// Low alchemy pays this fraction of an item's shop cost.
pub const LOW_ALCH_MULT: f64 = 0.4;
/// Magic level needed to cast High Level Alchemy.
pub const HIGH_ALCH_LEVEL: u32 = 55;
/// Magic level needed to cast Low Level Alchemy.
pub const LOW_ALCH_LEVEL: u32 = 21;
/// Magic XP per High Level Alchemy cast.
pub const HIGH_ALCH_XP: f64 = 65.0;
/// Magic XP per Low Level Alchemy cast.
pub const LOW_ALCH_XP: f64 = 31.0;
/// Nature runes consumed per alchemy cast, high or low.
pub const NATURE_RUNES_PER_ALCH: u64 = 1;
/// Fire runes per High Level Alchemy cast — zero when wielding a staff of fire.
pub const FIRE_RUNES_PER_HIGH_ALCH: u64 = 5;
/// Game ticks between casts (`p_delay(3)` plus the cast itself in practice).
pub const HIGH_ALCH_TICKS: u32 = 5;
/// Seconds per game tick on a normal-rate world.
pub const TICK_SECONDS: f64 = 0.6;
/// XP needed for level 55 Magic, the High Level Alchemy requirement.
pub const XP_FOR_LEVEL_55: f64 = 166_160.0;

/// Seconds between high alchemy casts.
pub fn seconds_per_high_alch() -> f64 {
    HIGH_ALCH_TICKS as f64 * TICK_SECONDS
}

/// Casts an uninterrupted alcher can manage in an hour.
pub fn high_alchs_per_hour() -> f64 {
    3_600.0 / seconds_per_high_alch()
}

/// GP a high alchemy cast pays for an item of the given shop cost.
///
/// Integer arithmetic, floored, minimum 1 — exactly as the game computes it.
pub fn high_alch_value(cost: u64) -> u64 {
    (cost * 6 / 10).max(1)
}

/// GP a low alchemy cast pays for an item of the given shop cost.
pub fn low_alch_value(cost: u64) -> u64 {
    (cost * 4 / 10).max(1)
}

/// The ratio between what high alchemy pays and what the pool floor charges.
///
/// Independent of the item: `0.6 x cost / (0.9 x 0.4 x cost)`. Anything above 1.0
/// is gross margin before rune costs, fees and price impact.
pub fn floor_to_high_alch_ratio() -> f64 {
    HIGH_ALCH_MULT / (0.9 * LOW_ALCH_MULT)
}

/// Profit from alching one item, given what it cost on chain and what the runes cost.
pub fn alch_profit(cost: u64, gp_paid: f64, rune_cost_gp: f64) -> f64 {
    high_alch_value(cost) as f64 - gp_paid - rune_cost_gp
}

/// The most GP worth paying for an item that will be high alched, to keep at
/// least `margin` fractional profit on the outlay.
pub fn max_price_for_margin(cost: u64, rune_cost_gp: f64, margin: f64) -> f64 {
    ((high_alch_value(cost) as f64 - rune_cost_gp) / (1.0 + margin)).max(0.0)
}

/// Hours of uninterrupted alching to reach level 55 Magic from zero, if every
/// cast were a high alch. Included because the level requirement, not the GP, is
/// what gates this strategy at the start.
pub fn hours_to_alch_level_from_scratch() -> f64 {
    XP_FOR_LEVEL_55 / HIGH_ALCH_XP / high_alchs_per_hour()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alch_values_match_the_game_formula() {
        // Lobster: cost 150 -> low 60, high 90. The registry agrees on lowalch 60.
        assert_eq!(low_alch_value(150), 60);
        assert_eq!(high_alch_value(150), 90);
        // Rounding is a floor, and never below 1.
        assert_eq!(high_alch_value(1), 1);
        assert_eq!(low_alch_value(1), 1);
        assert_eq!(high_alch_value(19), 11);
    }

    #[test]
    fn the_floor_sits_well_below_the_alch_payout() {
        let ratio = floor_to_high_alch_ratio();
        assert!((ratio - 5.0 / 3.0).abs() < 1e-12, "{ratio}");
    }

    #[test]
    fn profit_accounts_for_the_nature_rune() {
        // A 150 GP item bought at its 54 GP floor, one 7.2 GP nature rune.
        let profit = alch_profit(150, 54.0, 7.2);
        assert!((profit - 28.8).abs() < 1e-9, "{profit}");
        // A cheap item cannot carry the rune.
        assert!(alch_profit(20, 7.2, 7.2) < 0.0);
    }

    #[test]
    fn the_break_even_price_leaves_the_requested_margin() {
        let price = max_price_for_margin(150, 7.2, 0.25);
        assert!((price * 1.25 + 7.2 - 90.0).abs() < 1e-9, "{price}");
        // Never negative, even when the runes cost more than the payout.
        assert_eq!(max_price_for_margin(10, 100.0, 0.1), 0.0);
    }

    #[test]
    fn throughput_matches_a_three_second_cast() {
        assert!((seconds_per_high_alch() - 3.0).abs() < 1e-12);
        assert!((high_alchs_per_hour() - 1_200.0).abs() < 1e-9);
        // Roughly two days of solid casting to reach the level by alching alone,
        // which is why the level, not the capital, is the real barrier.
        let hours = hours_to_alch_level_from_scratch();
        assert!((2.0..3.0).contains(&hours), "{hours} hours");
    }
}

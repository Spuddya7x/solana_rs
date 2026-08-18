/**
 * Pickpocketing, and knowing when to stop.
 *
 * The engine side is in `skill_thieving/scripts/pickpocketing/pickpocket.rs2`
 * and its `pickpocket.dbrow`, and the Rust `mercantile_core::thieving` module
 * costs the whole thing. What lives here is the part the bot has to decide in
 * the moment: which pocket to pick, and when the next failure is the one that
 * kills the account.
 *
 * Three facts drive the policy:
 *
 * * **A failure costs a fixed hitpoint and eight ticks**, and it is announced —
 *   `~fail_pick_pocket` sets `%stunned`, so a stunned bot that keeps clicking
 *   just burns sends. Wait it out.
 * * **Passive regeneration is one hitpoint a minute** (`settimer(health_regen,
 *   100)`), against four hundred or more of damage an hour. Nothing here is
 *   sustainable without food.
 * * **Dying is not a rounding error.** `~damage_self` queues `player_death` the
 *   moment hitpoints reach zero, which drops the loot the run exists to collect.
 *   So the floor below is a hard stop, not a hint.
 */

import type { BotSDK } from '../../../sdk/index';

/** Ticks per second of game time. */
export const TICK_SECONDS = 0.6;

/** One row of the pickpocket table, as the bot needs it. */
export interface Target {
    /** Name for logs. */
    readonly name: string;
    /** NPC name to search for with `findNearbyNpc`. */
    readonly pattern: RegExp;
    /** Thieving level required. */
    readonly level: number;
    /** Hitpoints a failure costs. */
    readonly stunDamage: number;
    /** Ticks a failure locks the bot out for. */
    readonly stunTicks: number;
    /** `success_chance` low and high, for {@link successChance}. */
    readonly chance: readonly [number, number];
    /** Coins a success pays. */
    readonly coins: number;
    /** Where they stand. */
    readonly where: { readonly x: number; readonly z: number };
}

/**
 * The targets a Lumbridge-based account can actually reach.
 *
 * Deliberately short. The table has knights and heroes in it, but they are in
 * Ardougne behind a long walk, and this module exists to bootstrap an account
 * that has nothing — including no way to get to Ardougne.
 */
export const TARGETS: readonly Target[] = [
    {
        name: 'man/woman',
        // `man`, `man2`, `man3`, `woman`, `woman2`, `woman3` all render as
        // "Man"/"Woman", so match the display name rather than the script id.
        pattern: /^(man|woman)$/i,
        level: 1,
        stunDamage: 1,
        stunTicks: 8,
        chance: [180, 240],
        coins: 3,
        // A `man3` spawns three tiles from where the tutorial drops you.
        where: { x: 3221, z: 3219 },
    },
    {
        name: 'farmer',
        pattern: /^farmer$/i,
        level: 10,
        stunDamage: 1,
        stunTicks: 8,
        chance: [150, 240],
        coins: 9,
        // The field north of Lumbridge, 77 tiles from the castle.
        where: { x: 3227, z: 3290 },
    },
    {
        name: 'warrior woman',
        pattern: /^warrior woman$/i,
        level: 25,
        stunDamage: 2,
        stunTicks: 8,
        chance: [100, 240],
        coins: 18,
        // Varrock, 286 tiles out — only worth it once Thieving is high enough
        // that the walk amortises.
        where: { x: 3205, z: 3487 },
    },
];

/**
 * The engine's skill roll, from `ScriptOpcode.STAT_RANDOM`.
 *
 * `low` is the chance at level 1 and `high` the chance at 99, both out of 256,
 * linearly interpolated. A value of 256 or more cannot fail.
 */
export function statRandom(level: number, low: number, high: number): number {
    const l = Math.max(1, Math.min(99, level));
    const value = Math.floor((low * (99 - l)) / 98) + Math.floor((high * (l - 1)) / 98) + 1;
    return Math.min(256, Math.max(0, value)) / 256;
}

/** Chance one attempt on this target succeeds at a Thieving level. */
export function successChance(target: Target, level: number): number {
    return statRandom(level, target.chance[0], target.chance[1]);
}

/** Hitpoints an hour of picking this pocket costs. */
export function damagePerHour(target: Target, level: number): number {
    const p = successChance(target, level);
    const seconds = (p * 2 + (1 - p) * (1 + target.stunTicks)) * TICK_SECONDS;
    return (3600 / seconds) * (1 - p) * target.stunDamage;
}

/** The best target the account's Thieving level unlocks, among those we can reach. */
export function bestTarget(level: number, maxTiles = 200): Target {
    const reachable = TARGETS.filter((t) => t.level <= level).filter(
        (t) => Math.abs(t.where.x - 3222) + Math.abs(t.where.z - 3218) <= maxTiles,
    );
    // TARGETS is never empty, but the compiler cannot know that under
    // `noUncheckedIndexedAccess`, and a silent `undefined` here would send the
    // bot to pickpocket nothing.
    const chosen = reachable[reachable.length - 1] ?? TARGETS[0];
    if (!chosen) throw new Error('no pickpocket targets are defined');
    return chosen;
}

/**
 * How low hitpoints may go before the bot stops thieving.
 *
 * Two failures of headroom, never below two. A single failure's damage is known
 * exactly, but the bot can be mid-attempt when it checks, and a death costs the
 * whole inventory — so the margin is deliberately more than the arithmetic
 * demands.
 */
export function hitpointFloor(target: Target): number {
    return Math.max(2, target.stunDamage * 2 + 1);
}

/** Current and maximum hitpoints, or null if the skill has not arrived yet. */
export function hitpoints(sdk: BotSDK): { current: number; max: number } | null {
    const hp = sdk.getSkill('hitpoints') ?? sdk.getSkill('hp');
    if (!hp) return null;
    return { current: hp.level, max: hp.baseLevel ?? hp.level };
}

/** Whether it is safe to attempt another pickpocket right now. */
export function canContinue(sdk: BotSDK, target: Target): boolean {
    const hp = hitpoints(sdk);
    if (!hp) return false;
    return hp.current > hitpointFloor(target);
}

/**
 * Hitpoints of food the bot should carry home for a session of `minutes`.
 *
 * Passive regeneration is subtracted because it runs whether the bot is
 * thieving, walking or fishing — over a long trip it is not nothing.
 */
export function healingNeeded(target: Target, level: number, minutes: number): number {
    const hours = minutes / 60;
    return Math.max(0, (damagePerHour(target, level) - 60) * hours);
}

/**
 * Spell component ids and requirements.
 *
 * The BotSDK casts spells by interface component id. Those ids are positional:
 * the magic interface is 18, so component `1152 + n` is the n-th block in
 * `server/content/scripts/skill_magic/interfaces/magic.if`. The values below were
 * derived from that file and cross-checked against the SDK's own table
 * (`wind strike` = 1152, `confuse` = 1153, `curse` = 1161, `bind` = 1572 all
 * agree), so the alchemy ids are trustworthy even though the SDK does not list
 * them.
 *
 * Level, rune and XP data come from
 * `server/content/scripts/skill_magic/configs/magic_spells.dbrow`.
 */

/** Base component id of the magic interface. */
export const MAGIC_INTERFACE_BASE = 1152;

export interface SpellInfo {
    /** Interface component id, what `sendSpellOnItem` / `castSpell` want. */
    readonly component: number;
    /** Magic level needed. */
    readonly level: number;
    /** Magic XP per cast. */
    readonly xp: number;
    /** Runes consumed, by item name. Elemental runes are free with the matching staff. */
    readonly runes: Readonly<Record<string, number>>;
    /** Game ticks the cast occupies, from the spell script's `p_delay`. */
    readonly ticks: number;
}

export const SPELLS = {
    WIND_STRIKE: { component: 1152, level: 1, xp: 5.5, runes: { airrune: 1, mindrune: 1 }, ticks: 5 },
    WATER_STRIKE: { component: 1154, level: 5, xp: 7.5, runes: { waterrune: 1, airrune: 1, mindrune: 1 }, ticks: 5 },
    EARTH_STRIKE: { component: 1156, level: 9, xp: 9.5, runes: { earthrune: 2, airrune: 1, mindrune: 1 }, ticks: 5 },
    FIRE_STRIKE: { component: 1158, level: 13, xp: 11.5, runes: { firerune: 3, airrune: 2, mindrune: 1 }, ticks: 5 },
    CURSE: { component: 1161, level: 19, xp: 29, runes: { waterrune: 2, earthrune: 1, bodyrune: 1 }, ticks: 5 },
    /** 0.4 x cost, and the fastest cast in the game at 3 ticks. */
    LOW_ALCHEMY: { component: 1162, level: 21, xp: 31, runes: { naturerune: 1, firerune: 3 }, ticks: 3 },
    SUPERHEAT: { component: 1173, level: 43, xp: 53, runes: { naturerune: 1, firerune: 4 }, ticks: 5 },
    /** 0.6 x cost. The whole point of the exercise. */
    HIGH_ALCHEMY: { component: 1178, level: 55, xp: 65, runes: { naturerune: 1, firerune: 5 }, ticks: 5 },
} as const satisfies Record<string, SpellInfo>;

export type SpellName = keyof typeof SPELLS;

/** Seconds per game tick on a normal-rate world. */
export const TICK_SECONDS = 0.6;

/** Seconds one cast of a spell occupies. */
export function castSeconds(spell: SpellInfo): number {
    return spell.ticks * TICK_SECONDS;
}

/**
 * XP needed for a level, using the standard RuneScape curve.
 *
 * Level 55 (High Level Alchemy) is 166,160 XP; level 21 (Low Level Alchemy) is 5,018.
 */
export function xpForLevel(level: number): number {
    let points = 0;
    for (let l = 1; l < level; l++) {
        points += Math.floor(l + 300 * Math.pow(2, l / 7));
    }
    return Math.floor(points / 4);
}

/** Casts of `spell` needed to go from `fromXp` to `targetLevel`. */
export function castsToLevel(spell: SpellInfo, fromXp: number, targetLevel: number): number {
    const needed = xpForLevel(targetLevel) - fromXp;
    return needed <= 0 ? 0 : Math.ceil(needed / spell.xp);
}

/**
 * The best spell available at a given Magic level for *training*, given what the
 * bot can pay for.
 *
 * Low alchemy is the standout once it unlocks: 31 XP a cast at 3 ticks — the
 * fastest cast in the game — and it pays 0.4 x cost, which is more than the
 * 0.36 x cost an item takes to buy at its on-chain floor. Training pays for
 * itself from level 21 onwards, so the only real grind is 1 to 21.
 */
export function bestTrainingSpell(magicLevel: number, canAlch: boolean): SpellInfo {
    if (canAlch && magicLevel >= SPELLS.LOW_ALCHEMY.level) return SPELLS.LOW_ALCHEMY;
    if (magicLevel >= SPELLS.CURSE.level) return SPELLS.CURSE;
    if (magicLevel >= SPELLS.FIRE_STRIKE.level) return SPELLS.FIRE_STRIKE;
    if (magicLevel >= SPELLS.EARTH_STRIKE.level) return SPELLS.EARTH_STRIKE;
    if (magicLevel >= SPELLS.WATER_STRIKE.level) return SPELLS.WATER_STRIKE;
    return SPELLS.WIND_STRIKE;
}

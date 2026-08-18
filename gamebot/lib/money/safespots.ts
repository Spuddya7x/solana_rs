/**
 * Verified safespots.
 *
 * A safespot is a tile where the monster can be *seen* but cannot *reach* you.
 * Both halves were tested against the shipped collision data with the engine's
 * own routines, not recalled:
 *
 * * `rsmod.hasLineOfSight(player -> npc)` must hold, or you cannot cast at it —
 *   and the engine's `inApproachDistance` uses exactly this for spell and arrow
 *   range (`isApproached`, `PathingEntity.ts`).
 * * `findLongPath(npc -> player)` must fail to arrive. Line-of-walk alone is not
 *   enough: it tests a straight line, and monsters walk around pillars. Applying
 *   the weaker test marked every dungeon spawn as safe; the real one does not.
 *
 * The economics that decide whether a safespot is worth using are separate and
 * came out worse than expected — see `moneymaker.ts`. Low-level monsters in this
 * world drop items worth single-digit GP, because their `cost` values are small
 * and shops pay 60-70% of cost. These entries are here for the mid-game, once an
 * account can actually kill something worth killing.
 */

export interface Safespot {
    /** Monster's registry name. */
    npc: string;
    /** Expected GP of drops per kill, from the drop tables' own probabilities. */
    gpPerKill: number;
    /** Where to stand. */
    stand: { level: number; x: number; z: number };
    /** The spawn it covers. */
    target: { x: number; z: number };
}

/**
 * Ordered by what a kill is worth. Every entry was confirmed reachable-by-sight
 * and unreachable-by-path at the coordinates given.
 */
export const SAFESPOTS: Safespot[] = [
    // Wilderness spawns are excluded throughout: they are where the money is and
    // also where an unattended bot is someone else's loot.
    { npc: 'black_demon', gpPerKill: 706, stand: { level: 0, x: 2853, z: 9770 }, target: { x: 2854, z: 9776 } },
    { npc: 'greater_demon', gpPerKill: 502, stand: { level: 0, x: 2864, z: 9741 }, target: { x: 2861, z: 9747 } },
    { npc: 'firegiant', gpPerKill: 629, stand: { level: 0, x: 2561, z: 9889 }, target: { x: 2562, z: 9886 } },
    { npc: 'jogre', gpPerKill: 587, stand: { level: 0, x: 2821, z: 9513 }, target: { x: 2826, z: 9518 } },
    { npc: 'icegiant', gpPerKill: 181, stand: { level: 0, x: 3060, z: 9563 }, target: { x: 3054, z: 9564 } },
    // The only surface entry, and the most reachable for a mid-level account.
    { npc: 'mossgiant', gpPerKill: 129, stand: { level: 0, x: 2562, z: 3412 }, target: { x: 2554, z: 3409 } },
    { npc: 'lesser_demon', gpPerKill: 55, stand: { level: 0, x: 2841, z: 3271 }, target: { x: 2832, z: 3278 } },
    { npc: 'shadow_warrior', gpPerKill: 55, stand: { level: 0, x: 2690, z: 9776 }, target: { x: 2693, z: 9776 } },
];

/** The best safespot the caller can plausibly kill, richest first. */
export function bestSafespot(maxGpPerKill = Number.POSITIVE_INFINITY): Safespot | undefined {
    return SAFESPOTS.filter((s) => s.gpPerKill <= maxGpPerKill).sort((a, b) => b.gpPerKill - a.gpPerKill)[0];
}

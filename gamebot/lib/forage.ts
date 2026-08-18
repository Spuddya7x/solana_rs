/**
 * Making your own food, because nobody nearby sells any.
 *
 * The obvious answer to a thieving bot's healing bill is to buy bread. It does
 * not survive contact with the shop table: of the 64 counters a fresh account
 * can reach, exactly one stocks food worth eating, and it is Wydin's Food Store
 * in Port Sarim — 283 tiles away, selling cabbage that heals **one**. Bread is
 * in its list at base stock zero, so there is nothing on the shelf to buy.
 *
 * So the account fishes. The circuit is fixed by the map and happens to be
 * unusually tidy:
 *
 * ```
 *   Lumbridge (3222,3218)  ── 115 tiles ──▶  swamp shrimp (3267,3148)
 *          ▲                                        │
 *          └──── 55 ──── tree (3253,3194) ◀── 60 ───┘
 * ```
 *
 * The tree sits *exactly* on the straight line home — chopping, lighting and
 * cooking cost zero detour, which is why this route rather than any of the 196
 * other trees in the area. `(3265,3215)` is the equally-free alternative.
 *
 * Two engine details shape the loop and are worth stating because they are what
 * make it slow:
 *
 * * **Cooking 1 burns half the catch.** `cooking_generic_shrimp` rolls
 *   `successchance 128,512`, which is 129/256 at level 1 and does not reach a
 *   guaranteed 256 until level **34**. Below that, every cooked shrimp costs two
 *   raw ones.
 * * **A fire lasts 100–199 ticks** (`calc(100 + random(100))` in
 *   `~firemaking_success`), so one log is a minute or two of cooking — enough
 *   for an inventory, but not for two.
 *
 * The tutorial hands out the bronze axe, tinderbox and small fishing net at
 * `tutorial_complete`, so the whole circuit costs nothing to start.
 *
 * One SDK trap worth naming, because it is silent: **`waitForReady(n)` takes a
 * timeout in milliseconds, not ticks.** `waitForReady(6)` is "give up after six
 * milliseconds" and returns instantly, so a loop built on it spins instead of
 * pacing. `waitForTicks(n)` is the one that waits for game time to pass.
 */

import type { BotSDK } from '../../../sdk/index';
import type { BotActions } from '../../../sdk/actions';

/** Where the free shrimp are: Lumbridge Swamp, 115 tiles from the castle. */
export const SHRIMP_SPOT = { x: 3267, z: 3148 } as const;

/** A tree on the straight line from the swamp back to Lumbridge. */
export const TREE = { x: 3253, z: 3194 } as const;

/** Where the bot thieves, and where it walks back to. */
export const LUMBRIDGE = { x: 3222, z: 3218 } as const;

/** Hitpoints one cooked shrimp restores, from `consume_normal.dbrow`. */
export const SHRIMP_HEAL = 3;

/**
 * The tools the tutorial grants, all of which this loop needs.
 *
 * Matched against the **display** name, which is what the SDK reports and is
 * not the script id: `tutorial_complete` adds `net`, and the inventory calls it
 * *"Small fishing net"*. An `^net$` test never matches it, which silently
 * disables the whole forage loop.
 */
export const TOOLS = [/^small fishing net$/i, /^tinderbox$/i, /^bronze axe$/i] as const;

/** Result of one forage circuit. */
export interface ForageResult {
    /** Raw shrimp caught. */
    caught: number;
    /** Cooked shrimp carried home — the rest burned. */
    cooked: number;
    /** Hitpoints of healing that represents. */
    healing: number;
    /** Why the loop stopped. */
    reason: 'target-met' | 'inventory-full' | 'no-spot' | 'no-tools' | 'aborted';
}

/** Whether the account is carrying everything the circuit needs. */
export function hasTools(sdk: BotSDK): boolean {
    return TOOLS.every((tool) => sdk.countInventoryItems(tool) > 0);
}

/** Which of the required tools are missing, for a message worth reading. */
export function missingTools(sdk: BotSDK): string[] {
    return TOOLS.filter((tool) => sdk.countInventoryItems(tool) === 0).map((tool) =>
        tool.source.replace(/[\^$]/g, ''),
    );
}

/**
 * Fish until the inventory is full or `rawWanted` is reached.
 *
 * Shrimp come off `_saltfish` spots, which take the net on option 1. The spot
 * NPCs move — `fishing_movement.rs2` reshuffles them — so the pattern is
 * re-resolved every iteration rather than held.
 */
export async function fishShrimp(
    sdk: BotSDK,
    bot: BotActions,
    rawWanted: number,
    opts: { onProgress?: (caught: number) => void } = {},
): Promise<number> {
    const before = sdk.countInventoryItems(/^raw shrimps$/i);
    let idle = 0;

    while (sdk.countInventoryItems(/^raw shrimps$/i) - before < rawWanted) {
        if (sdk.getInventory().filter((s) => s).length >= 28) break;

        const spot = sdk.findNearbyNpc(/^fishing spot$/i);
        if (!spot) {
            // The spots shuffle between adjacent tiles; give the scan a moment
            // before concluding there is nothing here.
            if (++idle > 3) break;
            await sdk.waitForTicks(3);
            continue;
        }
        idle = 0;

        const caughtBefore = sdk.countInventoryItems(/^raw shrimps$/i);
        await bot.interactNpc(spot, /net|small net|fish/i);
        // A catch roll lands every five ticks; wait past one before judging.
        await sdk.waitForTicks(6);
        if (sdk.countInventoryItems(/^raw shrimps$/i) > caughtBefore) {
            opts.onProgress?.(sdk.countInventoryItems(/^raw shrimps$/i) - before);
        }
    }
    return sdk.countInventoryItems(/^raw shrimps$/i) - before;
}

/**
 * Chop one log, light it, and cook everything raw in the inventory on it.
 *
 * Returns the number of cooked shrimp that survived. Burnt fish are dropped:
 * they are worthless and the inventory slot is the scarce resource on the walk
 * home.
 */
export async function cookOnAFire(sdk: BotSDK, bot: BotActions): Promise<number> {
    const before = sdk.countInventoryItems(/^shrimps$/i);

    // A log first. Normal trees are level 0 with a bronze axe and never gate.
    // `chopTree` insists on *gaining* logs rather than on the tree vanishing,
    // which matters on a populated world where someone else fells it first.
    if (sdk.countInventoryItems(/^logs$/i) < 1) {
        for (let attempt = 0; attempt < 25; attempt++) {
            const chopped = await bot.chopTree(/^tree$/i);
            if (chopped.success) break;
            if (/no .*tree|not found/i.test(chopped.message ?? '')) break;
        }
    }
    if (sdk.countInventoryItems(/^logs$/i) < 1) return 0;

    // `[opheldu,tinderbox]` wants the tinderbox used on the logs; the engine
    // drops the log to the ground and lights it there. `burnLogs` waits on
    // Firemaking XP rather than on the animation, so a failed strike — the roll
    // is `stat_random(firemaking, 64, 512)` and misses often at level 1 — does
    // not read as a lit fire.
    for (let attempt = 0; attempt < 15; attempt++) {
        const burnt = await bot.burnLogs(/^logs$/i);
        if (burnt.success) break;
    }
    const fire = sdk.findNearbyLoc(/^fire$/i);
    if (!fire) return 0;

    // Cook the lot. Using the raw stack on the fire opens the count dialog; the
    // engine then works through the batch one roll at a time.
    while (sdk.countInventoryItems(/^raw shrimps$/i) > 0) {
        if (!sdk.findNearbyLoc(/^fire$/i)) break; // burnt out mid-batch
        const remaining = sdk.countInventoryItems(/^raw shrimps$/i);
        await bot.useItemOnLoc(/^raw shrimps$/i, fire);
        await sdk.sendCountDialog(remaining);
        // A cook is roughly four ticks a fish; wait for the batch, not a fish.
        await sdk.waitForTicks(4 * remaining + 4);
        if (sdk.countInventoryItems(/^raw shrimps$/i) === remaining) break; // stuck
    }

    await bot.dropItem(/^burnt fish$/i, 'all');
    return sdk.countInventoryItems(/^shrimps$/i) - before;
}

/**
 * One full circuit: walk to the swamp, fish, cook at the tree, come home.
 *
 * `healingWanted` is in hitpoints, not fish — the caller knows its damage rate,
 * not the burn rate. Because Cooking 1 loses half the catch, the raw target is
 * scaled by the measured cook chance rather than assumed one-for-one.
 */
export async function forage(
    sdk: BotSDK,
    bot: BotActions,
    healingWanted: number,
    opts: { log?: (message: string) => void } = {},
): Promise<ForageResult> {
    const log = opts.log ?? (() => {});
    if (!hasTools(sdk)) {
        return { caught: 0, cooked: 0, healing: 0, reason: 'no-tools' };
    }

    const cooking = sdk.getSkill('cooking')?.level ?? 1;
    const keep = statRandomLocal(cooking, 128, 512);
    const cookedWanted = Math.ceil(healingWanted / SHRIMP_HEAL);
    // Inventory is the binding constraint on a single trip, not the target.
    const rawWanted = Math.min(27, Math.ceil(cookedWanted / Math.max(keep, 0.1)));
    log(`forage: want ${cookedWanted} shrimp (${healingWanted} hp), keeping ${(keep * 100).toFixed(0)}% -> fishing ${rawWanted} raw`);

    await bot.walkTo(SHRIMP_SPOT.x, SHRIMP_SPOT.z);
    const caught = await fishShrimp(sdk, bot, rawWanted, {
        onProgress: (n) => log(`forage: ${n}/${rawWanted} raw`),
    });
    if (caught === 0) {
        return { caught: 0, cooked: 0, healing: 0, reason: 'no-spot' };
    }

    await bot.walkTo(TREE.x, TREE.z);
    const cooked = await cookOnAFire(sdk, bot);
    log(`forage: ${cooked} cooked from ${caught} raw`);

    await bot.walkTo(LUMBRIDGE.x, LUMBRIDGE.z);
    return {
        caught,
        cooked,
        healing: cooked * SHRIMP_HEAL,
        reason: cooked * SHRIMP_HEAL >= healingWanted ? 'target-met' : 'inventory-full',
    };
}

/** Local copy of the engine roll so this module does not depend on `thieving`. */
function statRandomLocal(level: number, low: number, high: number): number {
    const l = Math.max(1, Math.min(99, level));
    const value = Math.floor((low * (99 - l)) / 98) + Math.floor((high * (l - 1)) / 98) + 1;
    return Math.min(256, Math.max(0, value)) / 256;
}

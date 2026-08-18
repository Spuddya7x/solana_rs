/**
 * Casting alchemy on inventory items.
 *
 * The SDK has no alchemy helper — `enchantItem` is the only spell-on-item
 * porcelain — so this drives `sdk.sendSpellOnItem` directly and does its own
 * completion detection. Two things make that reliable:
 *
 * 1. **XP is the evidence.** A cast that lands always grants Magic XP; a cast
 *    that fails (no runes, wrong level, unalchable item) never does. Waiting on
 *    the XP counter is stricter than waiting on the item disappearing, which can
 *    also happen for unrelated reasons.
 * 2. **Slots are re-resolved every cast.** `sendSpellOnItem` addresses a slot,
 *    and the slot map shifts as items are consumed, so a loop built on a
 *    snapshot silently alches the wrong things.
 */

import type { BotActions } from '../../../sdk/actions';
import type { BotSDK } from '../../../sdk/index';
import type { InventoryItem } from '../../../sdk/types';
import { SPELLS, TICK_SECONDS, type SpellInfo } from './spells';

export interface AlchOptions {
    /** Which spell to cast. Defaults to high alchemy. */
    spell?: SpellInfo;
    /** Item names to cast on, in priority order. */
    targets: (string | RegExp)[];
    /** Stop after this many casts. */
    maxCasts?: number;
    /** Give up on a cast after this long. */
    timeoutMs?: number;
    /** Called after every successful cast. */
    onCast?: (progress: AlchProgress) => void;
}

export interface AlchProgress {
    casts: number;
    item: string;
    xpGained: number;
}

export interface AlchResult {
    casts: number;
    xpGained: number;
    /** Why the loop ended. */
    reason: 'done' | 'out_of_targets' | 'out_of_runes' | 'level_too_low' | 'max_casts' | 'stalled';
    message: string;
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Runes the bot is missing for one cast of `spell`, ignoring anything a staff covers. */
export function missingRunes(sdk: BotSDK, spell: SpellInfo): string[] {
    return Object.entries(spell.runes)
        .filter(([rune, needed]) => {
            // An elemental staff supplies its rune indefinitely, so only count
            // what the inventory has to provide.
            if (suppliedByStaff(sdk, rune)) return false;
            return sdk.countInventoryItems(runePattern(rune)) < needed;
        })
        .map(([rune]) => rune);
}

function runePattern(rune: string): RegExp {
    // "naturerune" -> /^nature rune$/i
    const spaced = rune.replace(/rune$/, ' rune');
    return new RegExp(`^${spaced}$`, 'i');
}

function suppliedByStaff(sdk: BotSDK, rune: string): boolean {
    const element = rune.replace(/rune$/, '');
    if (!['air', 'water', 'earth', 'fire'].includes(element)) return false;
    const state = sdk.getState();
    const worn = state?.equipment ?? [];
    return worn.some((item: InventoryItem) =>
        item?.name ? new RegExp(`staff of ${element}|${element} battlestaff`, 'i').test(item.name) : false,
    );
}

/**
 * Cast an alchemy spell on matching inventory items until they run out.
 *
 * Returns the reason it stopped rather than throwing: "no more runes" and "no
 * more items" are both normal ends to a production run, and the caller decides
 * whether to restock or finish.
 */
export async function alchInventory(
    sdk: BotSDK,
    _bot: BotActions,
    options: AlchOptions,
): Promise<AlchResult> {
    const spell = options.spell ?? SPELLS.HIGH_ALCHEMY;
    const timeoutMs = options.timeoutMs ?? 5_000;
    const maxCasts = options.maxCasts ?? Number.POSITIVE_INFINITY;

    const magic = sdk.getSkill('magic');
    if (!magic || magic.level < spell.level) {
        return {
            casts: 0,
            xpGained: 0,
            reason: 'level_too_low',
            message: `magic ${magic?.level ?? '?'} is below the level ${spell.level} this spell needs`,
        };
    }

    const startXp = magic.experience;
    let casts = 0;

    for (;;) {
        if (casts >= maxCasts) {
            return finish('max_casts', `stopped after ${casts} casts`);
        }
        const missing = missingRunes(sdk, spell);
        if (missing.length > 0) {
            return finish('out_of_runes', `out of ${missing.join(', ')}`);
        }
        // Re-resolve every cast: slots move as items are consumed.
        const target = findTarget(sdk, options.targets);
        if (!target) {
            return finish('out_of_targets', 'nothing left to alch');
        }

        const before = sdk.getSkill('magic')?.experience ?? 0;
        const dispatched = await sdk.sendSpellOnItem(target.slot, spell.component);
        if (!dispatched.success) {
            return finish('stalled', `the client refused the cast: ${dispatched.message}`);
        }

        const landed = await waitForXp(sdk, before, timeoutMs);
        if (!landed) {
            // One retry: a cast can be swallowed by a level-up or a random event.
            await _bot.dismissBlockingUI();
            const retried = await sdk.sendSpellOnItem(target.slot, spell.component);
            if (!retried.success || !(await waitForXp(sdk, before, timeoutMs))) {
                return finish('stalled', `no XP after casting on ${target.name}`);
            }
        }

        casts += 1;
        options.onCast?.({
            casts,
            item: target.name,
            xpGained: (sdk.getSkill('magic')?.experience ?? startXp) - startXp,
        });
    }

    function finish(reason: AlchResult['reason'], message: string): AlchResult {
        return {
            casts,
            xpGained: (sdk.getSkill('magic')?.experience ?? startXp) - startXp,
            reason,
            message,
        };
    }
}

function findTarget(sdk: BotSDK, targets: (string | RegExp)[]) {
    for (const pattern of targets) {
        const item = sdk.findInventoryItem(pattern);
        if (item) return item;
    }
    return null;
}

async function waitForXp(sdk: BotSDK, before: number, timeoutMs: number): Promise<boolean> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        await sleep(TICK_SECONDS * 1000 * 0.5);
        const now = sdk.getSkill('magic')?.experience ?? before;
        if (now > before) return true;
    }
    return false;
}

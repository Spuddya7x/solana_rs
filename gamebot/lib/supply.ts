/**
 * Keeping an account stocked with what it needs to train and alch.
 *
 * The runes for the whole training ladder come from open shops with no entry
 * requirements — Aubury in Varrock and Betty in Port Sarim both stock air,
 * water, earth, fire, mind, body, chaos and death runes, 1,000+ deep and
 * restocking. Nature runes are the exception and the bottleneck: nothing sells
 * them below 66 Magic, so they arrive by bridge from the chain side.
 */

import type { BotActions } from '../../../sdk/actions';
import type { BotSDK } from '../../../sdk/index';
import { NAV, capabilities, here, travelTo } from './nav';
import type { SpellInfo } from './spells';

/** Runes an open shop will sell to anyone. */
export const SHOP_RUNES = [
    'airrune',
    'waterrune',
    'earthrune',
    'firerune',
    'mindrune',
    'bodyrune',
    'chaosrune',
    'deathrune',
] as const;

/** "naturerune" -> /^nature rune$/i, matching the in-game item name. */
export function runePattern(rune: string): RegExp {
    return new RegExp(`^${rune.replace(/rune$/, ' rune')}$`, 'i');
}

export interface StockResult {
    success: boolean;
    message: string;
    bought: Record<string, number>;
    /** Runes no shop sells, which have to come from the chain. */
    unavailable: string[];
}

/**
 * Buy enough runes for `casts` casts of `spell`.
 *
 * Travels to the nearest rune shop first. A short fill is reported rather than
 * thrown: half the runes still trains, and the caller decides whether that is
 * enough to be worth starting.
 */
export async function stockRunes(
    bot: BotActions,
    sdk: BotSDK,
    spell: SpellInfo,
    casts: number,
    options: { travel?: boolean } = {},
): Promise<StockResult> {
    const bought: Record<string, number> = {};
    const unavailable: string[] = [];
    const needed: Record<string, number> = {};

    for (const [rune, per] of Object.entries(spell.runes)) {
        const want = per * casts - sdk.countInventoryItems(runePattern(rune));
        if (want <= 0) continue;
        if (!SHOP_RUNES.includes(rune as (typeof SHOP_RUNES)[number])) {
            unavailable.push(rune);
            continue;
        }
        needed[rune] = want;
    }

    if (Object.keys(needed).length === 0) {
        return {
            success: unavailable.length === 0,
            message: unavailable.length ? `no shop sells ${unavailable.join(', ')}` : 'already stocked',
            bought,
            unavailable,
        };
    }

    if (options.travel !== false) {
        const shop = NAV.nearestTagged(here(sdk), 'elemental_runes', capabilities(sdk));
        if (!shop) {
            return { success: false, message: 'no rune shop in the gazetteer', bought, unavailable };
        }
        const travelled = await travelTo(bot, sdk, shop.place.id);
        if (!travelled.success) {
            return { success: false, message: `could not reach the shop: ${travelled.message}`, bought, unavailable };
        }
    }

    const opened = await bot.openShop();
    if (!opened.success) {
        return { success: false, message: `could not open the shop: ${opened.message}`, bought, unavailable };
    }
    for (const [rune, want] of Object.entries(needed)) {
        // Shops sell in bounded lots; ask for the lot size the SDK supports and
        // repeat rather than assuming one call clears the whole order.
        let remaining = want;
        while (remaining > 0) {
            const lot = Math.min(remaining, 50);
            const result = await bot.buyFromShop(runePattern(rune), lot);
            const got = result.amountBought ?? (result.success ? lot : 0);
            bought[rune] = (bought[rune] ?? 0) + got;
            remaining -= got;
            if (got === 0) break;
        }
    }
    await bot.closeShop();

    const short = Object.entries(needed).filter(([rune, want]) => (bought[rune] ?? 0) < want);
    return {
        success: short.length === 0 && unavailable.length === 0,
        message: short.length
            ? `short on ${short.map(([r]) => r).join(', ')}`
            : `bought ${Object.entries(bought).map(([r, n]) => `${n} ${r}`).join(', ')}`,
        bought,
        unavailable,
    };
}

/** GP the runes for `casts` casts would cost, at the game's own shop values. */
export function runeBudget(spell: SpellInfo, casts: number): number {
    const cost: Record<string, number> = {
        airrune: 4, waterrune: 4, earthrune: 4, firerune: 4,
        mindrune: 3, bodyrune: 3, chaosrune: 15, deathrune: 30, naturerune: 20,
    };
    return Object.entries(spell.runes).reduce(
        (total, [rune, per]) => total + (cost[rune] ?? 0) * per * casts,
        0,
    );
}

/**
 * What a shop will actually pay.
 *
 * Ported from `server/content/scripts/shop/scripts/shop.rs2`:
 *
 * ```
 * calc_shop_value(cost, haggle, multiplier, diff):
 *   int5 = min(1000, max(-5000, diff * haggle))
 *   int5 = max(100, multiplier - int5)
 *   return scale(int5, 1000, cost)          // = int5 * cost / 1000
 * ```
 *
 * Two things fall out of that, and they decide the whole money-making plan:
 *
 * * **Shops buy at 60–70% of an item's cost** (`shop_buy_multiplier` is 600–700
 *   on real shopkeepers), while a Mercantile pool's floor is `0.9 x lowalch`,
 *   which is 36% of cost. Selling a floor-priced item to an NPC is the same
 *   1.67x that high alchemy pays — with no Magic level and no runes.
 * * **The price decays as you sell.** `diff` is how far the shop's stock has
 *   risen above its base, and each unit knocks `haggle/1000` off the multiplier,
 *   so 10 costs 1% of value per item and 30 costs 3%. That, not inventory space,
 *   is what caps a single visit.
 */

import type { Shop } from './world.generated';

/** The floor the formula clamps to: 10% of cost. */
export const MIN_MULTIPLIER = 100;

/**
 * GP a shop pays for one unit of an item, given how many have already been sold
 * into it this visit.
 *
 * A shop that stocks the item prices from its *base* stock, so selling into a
 * depleted shop pays more than the headline multiplier, and selling into a
 * well-stocked one pays less.
 */
export function sellPriceFor(shop: Shop, item: string, cost: number, alreadySold: number): number {
    const base = shop.stock[item] ?? 0;
    const diff = alreadySold - base;
    const adjustment = Math.min(1000, Math.max(-5000, diff * shop.haggle));
    const multiplier = Math.max(MIN_MULTIPLIER, shop.buyMultiplier - adjustment);
    return Math.floor((multiplier * cost) / 1000);
}

/** Total GP for selling `count` units, and the marginal price of the last one. */
export function sellTotal(
    shop: Shop,
    item: string,
    cost: number,
    count: number,
): { total: number; last: number } {
    let total = 0;
    let last = 0;
    for (let i = 0; i < count; i++) {
        last = sellPriceFor(shop, item, cost, i);
        total += last;
    }
    return { total, last };
}

/**
 * How many units are worth selling here before the price falls below `floor`.
 *
 * Used to decide when to walk to the next shop rather than dumping a whole
 * inventory into one and watching the last few sell for a tenth of their value.
 */
export function worthwhileCount(shop: Shop, item: string, cost: number, floor: number): number {
    let count = 0;
    while (count < 1_000 && sellPriceFor(shop, item, cost, count) >= floor) count++;
    return count;
}

/** Whether this shop will take the item at all. */
export function willBuy(shop: Shop, item: string): boolean {
    return shop.buysAnything || item in shop.stock;
}

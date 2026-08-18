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
 * * **A specialist buys at 60–95% of cost** (`shop_buy_multiplier` is 600–950
 *   on shopkeepers who deal in the item), while a Mercantile pool's floor is
 *   `0.9 x lowalch`, which is 36% of cost. Selling a floor-priced item to the
 *   right NPC beats the 1.67x that high alchemy pays — with no Magic level and
 *   no runes. A **general store** is a different animal: every one a fresh
 *   account can reach pays 400, i.e. 40% of cost, which is low-alchemy value
 *   and only 11% over the floor. General stores are for dumping loot, not for
 *   flipping.
 * * **The price decays as you sell.** `diff` is how far the shop's stock has
 *   risen above its base, and each unit knocks `haggle/1000` off the multiplier,
 *   so 10 costs 1% of value per item and 30 costs 3%. That, not inventory space,
 *   is what caps a single visit. At 400/haggle 30 the decay eats the whole 11%
 *   inside three units, which is why the general-store flip is not a trade.
 * * **The counter has to be reachable.** 53 of the 117 shops are not: upstairs,
 *   across water, or behind a door — or, for the Fremennik shops, behind a
 *   `%viking < ^viking_complete` check inside the shopkeeper's own script, with
 *   no door involved at all. `Shop.accessible` records the pathfinder's verdict
 *   and `Shop.barrier` says why not.
 */

import type { Shop } from './world.generated';

/** The floor the formula clamps to: 10% of cost. */
export const MIN_MULTIPLIER = 100;

/**
 * GP a shop pays for one unit, given the shop's current stock and how many have
 * already been sold this visit.
 *
 * `diff` is `current + sold - base`, so a depleted shop pays above the headline
 * multiplier and an overstocked one pays below it.
 */
export function sellPriceAt(
    shop: Shop,
    item: string,
    cost: number,
    currentStock: number,
    alreadySold: number,
): number {
    const diff = currentStock + alreadySold - (shop.stock[item] ?? 0);
    const adjustment = Math.min(1000, Math.max(-5000, diff * shop.haggle));
    const multiplier = Math.max(MIN_MULTIPLIER, shop.buyMultiplier - adjustment);
    return Math.floor((multiplier * cost) / 1000);
}

/**
 * The same, assuming the shop sits at its base stock — the neutral assumption
 * and the one to plan with. A depleted shop pays more (the adjustment clamps at
 * -5000, so up to 5.7x cost), but planning on that promises profits that only
 * exist if nobody has traded there recently.
 */
export function sellPriceFor(shop: Shop, item: string, cost: number, alreadySold: number): number {
    const base = shop.stock[item] ?? 0;
    const diff = alreadySold;
    void base;
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

/**
 * Alchemy economics, mirroring `mercantile-core`'s `alch` module on the Rust side.
 *
 * Both halves of this bot have to agree on what an item is worth, so the formulas
 * live in two places on purpose and are checked against the same source: the
 * game's own `alchemy.rs2`, which pays `max(floor(cost * 0.6), 1)` for a high alch
 * and `max(floor(cost * 0.4), 1)` for a low one.
 */

/** GP a high alchemy cast pays for an item of this shop cost. */
export function highAlchValue(cost: number): number {
    return Math.max(Math.floor((cost * 6) / 10), 1);
}

/** GP a low alchemy cast pays for an item of this shop cost. */
export function lowAlchValue(cost: number): number {
    return Math.max(Math.floor((cost * 4) / 10), 1);
}

/** The on-chain pool floor for an item: 0.9 x lowalch. */
export function poolFloor(cost: number): number {
    return lowAlchValue(cost) * 0.9;
}

/**
 * Profit from one high alch, given what the item cost on chain and the runes.
 *
 * At the pool floor this is `0.6c - 0.36c - runes = 0.24c - runes`, so an item
 * only pays for its own nature rune above roughly 30 GP of shop cost — and
 * comfortably above a few hundred.
 */
export function highAlchProfit(cost: number, gpPaid: number, runeCost: number): number {
    return highAlchValue(cost) - gpPaid - runeCost;
}

/** The same for a low alch, which is how levels 21 to 55 pay for themselves. */
export function lowAlchProfit(cost: number, gpPaid: number, runeCost: number): number {
    return lowAlchValue(cost) - gpPaid - runeCost;
}

/** The most GP worth paying for an item that will be high alched. */
export function maxPriceForHighAlch(cost: number, runeCost: number, margin = 0.25): number {
    return Math.max((highAlchValue(cost) - runeCost) / (1 + margin), 0);
}

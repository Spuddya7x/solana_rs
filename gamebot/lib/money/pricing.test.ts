/**
 * The shop price formula, checked against the game's own arithmetic.
 *
 * `server/content/scripts/shop/scripts/shop.rs2`:
 *   int5 = min(1000, max(-5000, diff * haggle))
 *   int5 = max(100, multiplier - int5)
 *   value = scale(int5, 1000, cost)     // a*c/b, so int5 * cost / 1000
 */

import { describe, expect, test } from 'bun:test';
import { MIN_MULTIPLIER, sellPriceFor, sellTotal, willBuy, worthwhileCount } from './pricing';
import type { Shop } from './world.generated';

const shop = (over: Partial<Shop> = {}): Shop => ({
    npc: 'shopkeeper',
    title: 'General Store',
    buyMultiplier: 600,
    haggle: 10,
    level: 0,
    x: 0,
    z: 0,
    stock: {},
    buysAnything: true,
    ...over,
});

describe('what a shop pays', () => {
    test('the headline multiplier is a percentage of cost', () => {
        // 600 / 1000 of a 1,000 gp item.
        expect(sellPriceFor(shop(), 'iron_platebody', 1_000, 0)).toBe(600);
        expect(sellPriceFor(shop({ buyMultiplier: 700 }), 'x', 1_000, 0)).toBe(700);
    });

    test('cost-1 items are worth literally nothing', () => {
        // Which is why the cow-hide run does not work here: hides, beef, bones,
        // feathers and raw chicken are all cost 1.
        expect(sellPriceFor(shop(), 'cow_hide', 1, 0)).toBe(0);
    });

    test('the price decays as the shop fills up', () => {
        const s = shop({ haggle: 10 });
        expect(sellPriceFor(s, 'x', 1_000, 0)).toBe(600);
        expect(sellPriceFor(s, 'x', 1_000, 10)).toBe(500);
        expect(sellPriceFor(s, 'x', 1_000, 25)).toBe(350);
    });

    test('a bigger haggle decays faster', () => {
        expect(sellPriceFor(shop({ haggle: 30 }), 'x', 1_000, 10)).toBe(300);
        expect(sellPriceFor(shop({ haggle: 10 }), 'x', 1_000, 10)).toBe(500);
    });

    test('the price floors at a tenth of cost, never lower', () => {
        const s = shop();
        expect(sellPriceFor(s, 'x', 1_000, 1_000)).toBe((MIN_MULTIPLIER * 1_000) / 1_000);
        expect(sellPriceFor(s, 'x', 1_000, 10_000)).toBe(100);
    });

    test('an understocked shop pays a premium for what it normally carries', () => {
        // diff = sold - base, so a shop with base stock 20 and none in hand
        // starts 20 steps *below* zero and pays above its headline rate.
        const stocked = shop({ stock: { firerune: 20 } });
        expect(sellPriceFor(stocked, 'firerune', 1_000, 0)).toBe(800);
        expect(sellPriceFor(shop(), 'firerune', 1_000, 0)).toBe(600);
    });

    test('the premium is capped, so a huge base stock is not free money', () => {
        // int5 is clamped at -5000 before subtraction, and the multiplier is
        // capped at 1000 by the min() on the adjustment.
        const huge = shop({ stock: { x: 100_000 } });
        expect(sellPriceFor(huge, 'x', 1_000, 0)).toBe(5_600);
    });
});

describe('sizing a sale', () => {
    test('the total is the sum of a falling series', () => {
        const { total, last } = sellTotal(shop(), 'x', 1_000, 3);
        expect(total).toBe(600 + 590 + 580);
        expect(last).toBe(580);
    });

    test('worthwhileCount stops when the price drops under the floor', () => {
        // 600 down to 300 in steps of 10 per unit is 30 units.
        expect(worthwhileCount(shop(), 'x', 1_000, 300)).toBe(31);
        expect(worthwhileCount(shop(), 'x', 1_000, 600)).toBe(1);
    });

    test('a shop that will not buy is excluded', () => {
        const picky = shop({ buysAnything: false, stock: { firerune: 10 } });
        expect(willBuy(picky, 'firerune')).toBe(true);
        expect(willBuy(picky, 'cow_hide')).toBe(false);
        expect(willBuy(shop(), 'cow_hide')).toBe(true);
    });
});

describe('the pool floor against the shop bid', () => {
    test('shops pay well above a pool floor, which is the whole arbitrage', () => {
        // Pool floor is 0.9 x lowalch = 0.36 x cost; shops pay 0.6-0.7 x cost.
        const cost = 10_000;
        const floor = 0.9 * Math.max(Math.floor((cost * 4) / 10), 1);
        expect(floor).toBe(3_600);
        expect(sellPriceFor(shop(), 'x', cost, 0)).toBe(6_000);
        expect(sellPriceFor(shop({ buyMultiplier: 700 }), 'x', cost, 0)).toBe(7_000);
        // The same 1.67x high alchemy pays, with no Magic level at all.
        expect(sellPriceFor(shop(), 'x', cost, 0) / floor).toBeCloseTo(5 / 3, 5);
    });
});

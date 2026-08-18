import { describe, expect, test } from 'bun:test';

import {
    TARGETS,
    bestTarget,
    damagePerHour,
    healingNeeded,
    hitpointFloor,
    statRandom,
    successChance,
} from './thieving';
import { SHRIMP_HEAL } from './forage';

// `noUncheckedIndexedAccess` makes every `TARGETS[n]` an optional, so name the
// three rows once here rather than asserting non-null at each use.
const [MAN, FARMER, WARRIOR] = TARGETS as readonly [
    (typeof TARGETS)[number],
    (typeof TARGETS)[number],
    (typeof TARGETS)[number],
];

describe('the engine roll', () => {
    test('collapses to low at level 1 and high at level 99', () => {
        expect(statRandom(1, 180, 240)).toBe(181 / 256);
        expect(statRandom(99, 180, 240)).toBe(241 / 256);
    });

    test('never exceeds certainty', () => {
        // Cooking shrimp is 128..512, which crosses 256 at level 34 and would
        // keep climbing past it without the clamp.
        expect(statRandom(34, 128, 512)).toBe(1);
        expect(statRandom(99, 128, 512)).toBe(1);
    });

    test('clamps levels outside 1..99', () => {
        expect(statRandom(0, 180, 240)).toBe(statRandom(1, 180, 240));
        expect(statRandom(120, 180, 240)).toBe(statRandom(99, 180, 240));
    });
});

describe('pickpocket targets', () => {
    test('a man succeeds seven times in ten at level 1', () => {
        expect(successChance(MAN, 1)).toBeCloseTo(0.707, 3);
    });

    test('passive regeneration cannot cover a man, let alone a farmer', () => {
        // 60 hp/hour is what standing still buys. This is the whole reason the
        // bot has a forage loop rather than an idle timer.
        expect(damagePerHour(MAN, 1)).toBeGreaterThan(400);
        expect(damagePerHour(FARMER, 10)).toBeGreaterThan(400);
    });

    test('the level gate is respected', () => {
        expect(bestTarget(1).name).toBe('man/woman');
        expect(bestTarget(9).name).toBe('man/woman');
        expect(bestTarget(10).name).toBe('farmer');
    });

    test('distant targets are excluded even when the level allows them', () => {
        // The warrior woman is in Varrock, 286 tiles out. A bootstrapping
        // account should not be sent there by default.
        expect(bestTarget(25).name).toBe('farmer');
        expect(bestTarget(25, 400).name).toBe('warrior woman');
    });
});

describe('the hitpoint floor', () => {
    test('leaves room for two failures', () => {
        expect(hitpointFloor(MAN)).toBe(3);
        expect(hitpointFloor(WARRIOR)).toBe(5);
    });

    test('is never low enough for a single hit to kill', () => {
        for (const target of TARGETS) {
            expect(hitpointFloor(target)).toBeGreaterThan(target.stunDamage);
        }
    });
});

describe('sizing a forage trip', () => {
    test('twelve minutes of farmers needs about thirty shrimp', () => {
        const hp = healingNeeded(FARMER, 10, 12);
        const shrimp = Math.ceil(hp / SHRIMP_HEAL);
        expect(shrimp).toBeGreaterThan(25);
        expect(shrimp).toBeLessThan(35);
    });

    test('a full inventory of shrimp does not last long', () => {
        // 25 slots x 3 hp is 75 hitpoints against a ~430/hour deficit, so a trip
        // buys about ten minutes. This is the loop's central problem, not a bug.
        const perHour = damagePerHour(FARMER, 10) - 60;
        const minutes = (25 * SHRIMP_HEAL) / perHour * 60;
        expect(minutes).toBeLessThan(15);
    });

    test('a zero-length session needs no food', () => {
        expect(healingNeeded(MAN, 1, 0)).toBe(0);
    });
});

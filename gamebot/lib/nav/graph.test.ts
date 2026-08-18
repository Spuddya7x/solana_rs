/**
 * Router tests. Pure logic — no game, no pathfinder, no network.
 *
 * Run from a mercantile checkout: `bun test bots/<name>/lib/nav/`
 */

import { describe, expect, test } from 'bun:test';
import { NavGraph, estimateWalkCost, meets, type WalkCost } from './graph';
import type { Capabilities, Link, Place } from './types';

const places: Place[] = [
    { id: 'town', name: 'Town', level: 0, x: 100, z: 100, tags: ['city', 'bank'] },
    { id: 'shop', name: 'Shop', level: 0, x: 110, z: 100, tags: ['shop'] },
    { id: 'far_town', name: 'Far town', level: 0, x: 900, z: 900, tags: ['city', 'bank'] },
    { id: 'cellar_door', name: 'Cellar door', level: 0, x: 120, z: 100, tags: [] },
    { id: 'cellar', name: 'Cellar', level: 1, x: 120, z: 100, tags: ['undead'] },
];

const links: Link[] = [
    {
        from: '*',
        to: 'far_town',
        kind: 'teleport',
        cost: 8,
        spellComponent: 1164,
        requires: { magicLevel: 25, runes: { lawrune: 1 } },
    },
    {
        from: 'cellar_door',
        to: 'cellar',
        kind: 'object',
        cost: 5,
        object: { name: /trapdoor/i },
        requires: { magicLevel: 10 },
    },
];

const graph = new NavGraph(places, links);
const at = (x: number, z: number, level = 0) => ({ x, z, level });

const caps = (over: Partial<Capabilities> = {}): Capabilities => ({
    magicLevel: 99,
    runes: () => 100,
    gp: 10_000,
    ...over,
});

describe('requirements', () => {
    test('a missing level, rune or coin blocks a link', () => {
        expect(meets({ magicLevel: 25 }, caps({ magicLevel: 24 }))).toBe(false);
        expect(meets({ magicLevel: 25 }, caps({ magicLevel: 25 }))).toBe(true);
        expect(meets({ runes: { lawrune: 2 } }, caps({ runes: () => 1 }))).toBe(false);
        expect(meets({ gp: 30 }, caps({ gp: 29 }))).toBe(false);
        expect(meets(undefined, caps({ magicLevel: 1 }))).toBe(true);
    });
});

describe('walk costs', () => {
    test('different levels are never walkable', () => {
        expect(estimateWalkCost(places[0], places[4])).toBeNull();
    });

    test('cost is chebyshev distance, the way the game measures movement', () => {
        expect(estimateWalkCost(places[0], places[1])).toBe(10);
    });
});

describe('routing', () => {
    test('a nearby place is one walk', () => {
        const route = graph.route(at(100, 100), 'shop', caps())!;
        expect(route.steps).toHaveLength(1);
        expect(route.steps[0].kind).toBe('walk');
        expect(route.cost).toBe(10);
    });

    test('a teleport beats a long walk when the runes are there', () => {
        const route = graph.route(at(100, 100), 'far_town', caps())!;
        expect(route.steps[0].kind).toBe('teleport');
        expect(route.cost).toBe(8);
    });

    test('without the runes the same trip becomes a walk', () => {
        const route = graph.route(at(100, 100), 'far_town', caps({ runes: () => 0 }))!;
        expect(route.steps.every((step) => step.kind === 'walk')).toBe(true);
        expect(route.cost).toBe(800);
    });

    test('a level change routes through the object that makes it', () => {
        const route = graph.route(at(100, 100), 'cellar', caps())!;
        expect(route.steps.map((s) => s.kind)).toEqual(['walk', 'object']);
        expect(route.steps[1].link?.object?.name).toEqual(/trapdoor/i);
    });

    test('an unreachable place returns null rather than a wrong route', () => {
        // Magic 1 fails the trapdoor requirement, and no walk crosses levels.
        expect(graph.route(at(100, 100), 'cellar', caps({ magicLevel: 1 }))).toBeNull();
    });

    test('being already there is a zero-step route', () => {
        const route = graph.route(at(110, 100), 'shop', caps())!;
        expect(route.cost).toBe(0);
        expect(route.steps).toHaveLength(0);
    });

    test('an unknown destination is an error, not a silent failure', () => {
        expect(() => graph.route(at(0, 0), 'atlantis', caps())).toThrow('unknown place');
    });
});

describe('nearest by tag', () => {
    test('picks the cheapest tagged place, not the first', () => {
        const nearest = graph.nearestTagged(at(880, 880), 'bank', caps({ runes: () => 0 }))!;
        expect(nearest.place.id).toBe('far_town');
    });

    test('a teleport can make a distant bank the nearest one', () => {
        const nearest = graph.nearestTagged(at(400, 400), 'bank', caps())!;
        expect(nearest.place.id).toBe('far_town');
        expect(nearest.route.steps[0].kind).toBe('teleport');
    });

    test('an absent tag yields nothing', () => {
        expect(graph.nearestTagged(at(100, 100), 'volcano', caps())).toBeNull();
    });
});

describe('the shipped gazetteer', () => {
    test('every link points at a place that exists', async () => {
        const { PLACES } = await import('./places');
        const { LINKS } = await import('./links');
        const ids = new Set(PLACES.map((p) => p.id));
        for (const link of LINKS) {
            expect(ids.has(link.to)).toBe(true);
            if (link.from !== '*') expect(ids.has(link.from)).toBe(true);
        }
    });

    test('place ids are unique', async () => {
        const { PLACES } = await import('./places');
        expect(new Set(PLACES.map((p) => p.id)).size).toBe(PLACES.length);
    });

    test('the tags the bot routes on are all populated', async () => {
        const { PLACES } = await import('./places');
        for (const tag of ['bank', 'undead', 'chickens', 'runes', 'nature_runes']) {
            expect(PLACES.some((p) => p.tags.includes(tag))).toBe(true);
        }
    });

    test('a custom walk cost is used instead of the estimate', () => {
        const impassable: WalkCost = () => null;
        expect(graph.route(at(100, 100), 'shop', caps({ runes: () => 0 }), impassable)).toBeNull();
    });
});

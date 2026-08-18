/**
 * Regenerate the gazetteer from the game's own data.
 *
 * Run from a mercantile checkout with this directory copied into `bots/<name>/`:
 *   bun bots/<name>/tools/build-places.ts > bots/<name>/lib/nav/places.generated.ts
 *
 * Sources, all inside `server/content`:
 *   maps/labels.txt   — the map's own region labels: `=Name,x,z,level`
 *   maps/*.jm2        — `==== NPC ====` and `==== LOC ====` spawn sections,
 *                       world coords being `mapX * 64 + localX`
 *   pack/npc.pack     — npc id to debugname
 *   pack/loc.pack     — loc id to debugname
 *
 * Nothing here is remembered or guessed; if the world data changes, rerun it.
 */

import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const CONTENT = join(import.meta.dir, '..', '..', '..', 'server', 'content');
const MAPS = join(CONTENT, 'maps');

interface Spawn {
    id: number;
    level: number;
    x: number;
    z: number;
}

function packIds(file: string): Map<string, number> {
    const ids = new Map<string, number>();
    for (const line of readFileSync(join(CONTENT, 'pack', file), 'utf8').split('\n')) {
        const m = line.match(/^(\d+)=(\S+)/);
        if (m) ids.set(m[2], Number(m[1]));
    }
    return ids;
}

/** Every NPC and LOC spawn across every map square, in world coordinates. */
function readSpawns(): { npcs: Spawn[]; locs: Spawn[] } {
    const npcs: Spawn[] = [];
    const locs: Spawn[] = [];
    for (const file of readdirSync(MAPS)) {
        const m = file.match(/^m(\d+)_(\d+)\.jm2$/);
        if (!m) continue;
        const mapX = Number(m[1]);
        const mapZ = Number(m[2]);
        let section = '';
        for (const line of readFileSync(join(MAPS, file), 'utf8').split('\n')) {
            const t = line.trim();
            if (t.startsWith('====')) {
                section = t.replace(/=/g, '').trim();
                continue;
            }
            const mm = t.match(/^(\d+)\s+(\d+)\s+(\d+):\s*(\d+)/);
            if (!mm) continue;
            const spawn: Spawn = {
                level: Number(mm[1]),
                x: mapX * 64 + Number(mm[2]),
                z: mapZ * 64 + Number(mm[3]),
                id: Number(mm[4]),
            };
            if (section === 'NPC') npcs.push(spawn);
            else if (section === 'LOC') locs.push(spawn);
        }
    }
    return { npcs, locs };
}

/** Group spawns that sit within `radius` tiles of each other on one level. */
function cluster(spawns: Spawn[], radius = 12): Spawn[] {
    const groups: Spawn[][] = [];
    for (const spawn of spawns) {
        const near = groups.find((g) =>
            g.some(
                (o) =>
                    o.level === spawn.level &&
                    Math.abs(o.x - spawn.x) <= radius &&
                    Math.abs(o.z - spawn.z) <= radius,
            ),
        );
        if (near) near.push(spawn);
        else groups.push([spawn]);
    }
    // Represent each cluster by its centroid, rounded to a tile.
    return groups.map((g) => ({
        id: g[0].id,
        level: g[0].level,
        x: Math.round(g.reduce((s, o) => s + o.x, 0) / g.length),
        z: Math.round(g.reduce((s, o) => s + o.z, 0) / g.length),
    }));
}

const npcIds = packIds('npc.pack');
const locIds = packIds('loc.pack');
const { npcs, locs } = readSpawns();

const spawnsOf = (names: string[], from: Spawn[] = npcs, ids = npcIds) => {
    const wanted = new Set(names.map((n) => ids.get(n)).filter((v): v is number => v !== undefined));
    return from.filter((s) => wanted.has(s.id));
};

const labels = readFileSync(join(MAPS, 'labels.txt'), 'utf8')
    .split('\n')
    .map((line) => line.match(/^=([^,]+),(\d+),(\d+),(\d+)/))
    .filter((m): m is RegExpMatchArray => m !== null)
    .map((m) => ({ name: m[1].replace(/\//g, ' '), x: Number(m[2]), z: Number(m[3]) }));

const banks = cluster(spawnsOf(['banker', 'banker2', 'banker3', 'banker4']));
const undead = cluster(spawnsOf(['skeleton_unarmed', 'skeleton_armed', 'zombie_unarmed', 'zombie2']), 20);
const chickens = cluster(spawnsOf(['chicken']), 20);
const clerks = cluster(spawnsOf(['exchange_clerk']));

console.error(
    `labels ${labels.length}, banks ${banks.length}, undead sites ${undead.length}, ` +
        `chicken farms ${chickens.length}, clerks ${clerks.length}`,
);

const named = (prefix: string, list: Spawn[], tags: string[]) =>
    list.map((s, i) => ({
        id: `${prefix}_${i + 1}`,
        name: `${prefix.replace(/_/g, ' ')} ${i + 1}`,
        level: s.level,
        x: s.x,
        z: s.z,
        tags,
    }));

const places = [
    ...labels.map((l) => ({
        id: l.name.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_|_$/g, ''),
        name: l.name,
        level: 0,
        x: l.x,
        z: l.z,
        tags: ['region'],
    })),
    ...named('bank', banks, ['bank', 'clerk']),
    ...named('undead', undead, ['undead', 'training']),
    ...named('chickens', chickens, ['chickens', 'training']),
];

// The map has several places sharing a name — three "Agility Training Area"s,
// two "Chaos Temple"s — and they are genuinely different locations, so suffix
// rather than drop.
const seen = new Map<string, number>();
for (const p of places) {
    const count = (seen.get(p.id) ?? 0) + 1;
    seen.set(p.id, count);
    if (count > 1) p.id = `${p.id}_${count}`;
}

console.log('// Generated by tools/build-places.ts — do not edit by hand.');
console.log("import type { Place } from './types';\n");
console.log('export const GENERATED_PLACES: Place[] = [');
for (const p of places) {
    console.log(
        `    { id: ${JSON.stringify(p.id)}, name: ${JSON.stringify(p.name)}, level: ${p.level}, ` +
            `x: ${p.x}, z: ${p.z}, tags: ${JSON.stringify(p.tags)} },`,
    );
}
console.log('];');

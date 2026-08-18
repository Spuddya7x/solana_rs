/**
 * Generate the money-maker's world data: what lies on the ground, and who buys it.
 *
 * Two things a fresh account needs and cannot work out for itself:
 *
 * **Ground spawns.** The map's `==== OBJ ====` sections list every item that
 * sits on the floor and respawns — free inventory, no levels, no combat. Items
 * respawn after `respawnrate` ticks (default 100, so a minute).
 *
 * **Shops.** `shop_buy_multiplier` on the shopkeeper NPC decides what selling
 * pays: `cost x multiplier / 1000`, so 600 means 60% of an item's value and 700
 * means 70%. The multiplier lives on the NPC, its stock in a `.inv`, and its
 * position in the map NPC sections, so all three are joined here.
 *
 * Wilderness spawns are excluded, and shops there are flagged. That is where the
 * valuable ones are, and also where an unattended bot is somebody else's loot.
 *
 *   bun bots/<name>/tools/build-money.ts        > lib/money/world.generated.ts
 *   bun bots/<name>/tools/build-money.ts --json > crates/mercantile-core/data/shops.json
 *
 * The `--json` form is what the Rust side reads, so `mercbot shop-flip` prices
 * the exit against the same shop table the game bot sells into.
 */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import * as rsmod from '../../../server/vendor/rsmod-pathfinder';
import { findLongPath, initPathfinding, type DoorInfo } from '../../../sdk/pathfinding';

const ROOT = join(import.meta.dir, '..', '..', '..');
const CONTENT = join(ROOT, 'server', 'content');
const MAPS = join(CONTENT, 'maps');

/**
 * The wilderness, from the game's own `wilderness_zones.dbrow`:
 * `0_46_55_0_0` to `3_52_99_63_63` on the surface, and `0_46_155_0_0` to
 * `0_52_199_63_63` underground. It is bounded in **x as well as z** — filtering
 * on z alone wrongly condemns Rellekka and the whole north-west.
 */
const WILDERNESS = {
    minX: 46 * 64,
    maxX: 52 * 64 + 63,
    surface: { minZ: 55 * 64, maxZ: 99 * 64 + 63 },
    underground: { minZ: 155 * 64, maxZ: 199 * 64 + 63 },
};
const UNDERGROUND_Z_OFFSET = 6400;

function inWilderness(x: number, z: number): boolean {
    if (x < WILDERNESS.minX || x > WILDERNESS.maxX) return false;
    const { surface, underground } = WILDERNESS;
    return (z >= surface.minZ && z <= surface.maxZ) || (z >= underground.minZ && z <= underground.maxZ);
}
/** ObjType default when a config does not override it: 100 ticks, one minute. */
const DEFAULT_RESPAWN_TICKS = 100;

function walk(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir)) {
        const path = join(dir, entry);
        if (statSync(path).isDirectory()) walk(path, out);
        else out.push(path);
    }
    return out;
}

const scriptFiles = walk(join(CONTENT, 'scripts')).filter((p) => !p.includes('/_unpack/'));

/** Parse `[name] ... key=value` config blocks out of every file with an extension. */
function blocks(ext: string): Map<string, string> {
    const found = new Map<string, string>();
    for (const path of scriptFiles.filter((p) => p.endsWith(ext))) {
        for (const block of readFileSync(path, 'utf8').split(/\n(?=\[)/)) {
            const name = block.match(/^\[(\w+)\]/);
            if (name) found.set(name[1], block);
        }
    }
    return found;
}

const objBlocks = blocks('.obj');
const npcBlocks = blocks('.npc');
const invBlocks = blocks('.inv');

const field = (block: string | undefined, key: string): string | undefined =>
    block?.match(new RegExp(`\\n${key}=(.+)`))?.[1]?.trim();
const intField = (block: string | undefined, key: string, fallback: number): number => {
    const raw = field(block, key);
    return raw === undefined ? fallback : Number(raw);
};
const param = (block: string | undefined, key: string): string | undefined =>
    block?.match(new RegExp(`param=${key},(.+)`))?.[1]?.trim();

const objIds = new Map<number, string>();
for (const line of readFileSync(join(CONTENT, 'pack', 'obj.pack'), 'utf8').split('\n')) {
    const m = line.match(/^(\d+)=(\S+)/);
    if (m) objIds.set(Number(m[1]), m[2]);
}
const npcIds = new Map<number, string>();
for (const line of readFileSync(join(CONTENT, 'pack', 'npc.pack'), 'utf8').split('\n')) {
    const m = line.match(/^(\d+)=(\S+)/);
    if (m) npcIds.set(Number(m[1]), m[2]);
}

interface Spawn {
    item: string;
    cost: number;
    level: number;
    x: number;
    z: number;
    respawnTicks: number;
}
interface ShopNpc {
    npc: string;
    title: string;
    buyMultiplier: number;
    haggle: number;
    level: number;
    x: number;
    z: number;
    /** Items the shop keeps in stock, and how many it holds at base. */
    stock: Record<string, number>;
    /** Whether it will buy items it does not stock. */
    buysAnything: boolean;
    /** Outside the wilderness. The best-paying shops are not: an unattended bot
     *  walking to the Bandit Camp is somebody else's loot. */
    safe: boolean;
    /** Whether a fresh account can actually walk here from Lumbridge. */
    accessible: boolean;
    /** Why not, when it cannot: `upstairs`, `unreachable`, or a gate's requirement. */
    barrier: string | null;
    /** Tiles of walking from Lumbridge — one tick each, half that running. */
    walkTiles: number | null;
}

const spawns: Spawn[] = [];
const shopNpcs = new Map<string, { level: number; x: number; z: number }>();

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
        const g = t.match(/^(\d+)\s+(\d+)\s+(\d+):\s*(\d+)/);
        if (!g) continue;
        const level = Number(g[1]);
        const x = mapX * 64 + Number(g[2]);
        const z = mapZ * 64 + Number(g[3]);
        const id = Number(g[4]);

        if (section === 'OBJ') {
            const item = objIds.get(id);
            if (!item) continue;
            if (inWilderness(x, z)) continue;
            const block = objBlocks.get(item);
            const cost = intField(block, 'cost', 1);
            if (cost < 50) continue;
            spawns.push({
                item,
                cost,
                level,
                x,
                z,
                respawnTicks: intField(block, 'respawnrate', DEFAULT_RESPAWN_TICKS),
            });
        } else if (section === 'NPC') {
            const npc = npcIds.get(id);
            if (npc && param(npcBlocks.get(npc), 'shop_buy_multiplier')) {
                shopNpcs.set(npc, { level, x, z });
            }
        }
    }
}

/** Group spawns that can be collected from one stop. */
function cluster(list: Spawn[], radius: number): Spawn[][] {
    const groups: Spawn[][] = [];
    for (const spawn of list) {
        const near = groups.find((g) =>
            g.some((o) => o.level === spawn.level && Math.abs(o.x - spawn.x) <= radius && Math.abs(o.z - spawn.z) <= radius),
        );
        if (near) near.push(spawn);
        else groups.push([spawn]);
    }
    return groups;
}

const clusters = cluster(spawns, 12)
    .map((group) => ({
        level: group[0].level,
        x: Math.round(group.reduce((s, o) => s + o.x, 0) / group.length),
        z: Math.round(group.reduce((s, o) => s + o.z, 0) / group.length),
        items: group.map((o) => ({ item: o.item, cost: o.cost, x: o.x, z: o.z, respawnTicks: o.respawnTicks })),
        value: group.reduce((s, o) => s + o.cost, 0),
    }))
    .filter((c) => c.value >= 100)
    .sort((a, b) => b.value - a.value);

const shops: ShopNpc[] = [];
for (const [npc, where] of shopNpcs) {
    const block = npcBlocks.get(npc);
    const invName = param(block, 'owned_shop');
    const inv = invName ? invBlocks.get(invName) : undefined;
    const stock: Record<string, number> = {};
    for (const line of (inv ?? '').split('\n')) {
        const s = line.match(/^stock\d+=(\w+),(\d+)/);
        if (s) stock[s[1]] = Number(s[2]);
    }
    shops.push({
        npc,
        title: (param(block, 'shop_title') ?? 'Shop').replace(/"/g, ''),
        buyMultiplier: Number(param(block, 'shop_buy_multiplier') ?? 600),
        haggle: Number(param(block, 'shop_delta') ?? 10),
        level: where.level,
        x: where.x,
        z: where.z,
        stock,
        buysAnything: field(inv, 'allstock') === 'yes',
        safe: !inWilderness(where.x, where.z),
        // Filled in by the access pass below.
        accessible: true,
        barrier: null,
        walkTiles: null,
    });
}
// ── access: can a fresh account actually get to the counter? ─────────────────
//
// Three separate barriers, and the shop-flip scanner recommended a shop behind
// two of them before this existed (the Legends Guild store, which is upstairs
// *and* behind a quest door).
//
//  upstairs     the router walks one level; a shop on level 1+ needs stairs
//  unreachable  no path at all — an island, or somewhere needing a boat
//  gated        a path exists, but only through a door that checks a skill,
//               quest-point total or quest variable
//
// The gate test is done by blocking every gated door in the collision map and
// re-running the pathfinder: if that makes the shop unreachable, the shop is
// behind the gate. That is the property we care about, and it needs no map of
// which building is which.
// initPathfinding logs its progress to stdout, which would corrupt the JSON
// this script emits there.
{
    const log = console.log;
    console.log = (...args: unknown[]) => console.error(...args);
    initPathfinding();
    console.log = log;
}

/** Where a fresh account stands after the tutorial: Lumbridge. */
const HUB = { level: 0, x: 3222, z: 3218 };

const locIds = new Map<string, number>();
for (const line of readFileSync(join(CONTENT, 'pack', 'loc.pack'), 'utf8').split('\n')) {
    const m = line.match(/^(\d+)=(\S+)/);
    if (m) locIds.set(m[2], Number(m[1]));
}

/** Requirement a door imposes, read from the first lines of its `oploc` handler. */
function gateRequirement(block: string): string | null {
    const head = block.split('\n').slice(0, 14).join('\n');
    const skill = head.match(/if\s*\(\s*stat\((\w+)\)\s*<\s*(\d+)/);
    if (skill) return `${skill[1]} ${skill[2]}`;
    const qp = head.match(/if\s*\(\s*%qp\s*<\s*(\d+)/);
    if (qp) return `${qp[1]} quest points`;
    const quest = head.match(/if\s*\(\s*%(\w*quest\w*|\w+)\s*<\s*\^(\w+_complete)/);
    if (quest) return `quest: ${quest[1]}`;
    return null;
}

// Gates come in two shapes, and only one of them is a door.
//
// The Fremennik shops are wide open on the map — the check lives inside the
// shopkeeper's own script (`%viking < ^viking_complete`), so no amount of
// blocking doors reveals them. Reading the NPC's handler catches those, and it
// is the more direct signal: the shop refuses you at the counter.
function npcGate(npc: string): string | null {
    for (const path of scriptFiles.filter((p) => p.endsWith('.rs2'))) {
        const text = readFileSync(path, 'utf8');
        if (!text.includes(npc)) continue;
        for (const block of text.split(/\n(?=\[)/)) {
            if (!new RegExp(`^\\[opnpc\\d*,\\s*_?${npc}\\]`).test(block)) continue;
            const quest = block.match(/%(\w+)\s*<\s*\^(\w+)_complete/);
            if (quest) return `quest: ${quest[1]}`;
            const skill = block.match(/stat\((\w+)\)\s*<\s*(\d+)/);
            if (skill) return `${skill[1]} ${skill[2]}`;
            const qp = block.match(/%qp\s*<\s*(\d+)/);
            if (qp) return `${qp[1]} quest points`;
        }
    }
    return null;
}

const gatedLocIds = new Map<number, string>();
for (const path of scriptFiles.filter((p) => p.endsWith('.rs2'))) {
    for (const block of readFileSync(path, 'utf8').split(/\n(?=\[)/)) {
        const head = block.match(/^\[oploc[12],\s*(\w+)\]/);
        if (!head) continue;
        const requirement = gateRequirement(block);
        const id = locIds.get(head[1]);
        if (requirement && id !== undefined) gatedLocIds.set(id, requirement);
    }
}

/** Every placed instance of a gated loc, as a blockable door. */
const gatedDoors: Array<DoorInfo & { requirement: string }> = [];
for (const file of readdirSync(MAPS)) {
    const m = file.match(/^m(\d+)_(\d+)\.jm2$/);
    if (!m) continue;
    let section = '';
    for (const line of readFileSync(join(MAPS, file), 'utf8').split('\n')) {
        const t = line.trim();
        if (t.startsWith('====')) { section = t.replace(/=/g, '').trim(); continue; }
        if (section !== 'LOC') continue;
        const g = t.match(/^(\d+)\s+(\d+)\s+(\d+):\s*(\d+)(?:\s+(\d+))?(?:\s+(\d+))?/);
        if (!g) continue;
        const requirement = gatedLocIds.get(Number(g[4]));
        if (!requirement) continue;
        gatedDoors.push({
            level: Number(g[1]),
            x: Number(m[1]) * 64 + Number(g[2]),
            z: Number(m[2]) * 64 + Number(g[3]),
            shape: Number(g[5] ?? 0),
            angle: Number(g[6] ?? 0),
            blockrange: false,
            requirement,
        });
    }
}

/** Walk distance in tiles, or null if the destination cannot be reached. */
function walkTiles(to: { level: number; x: number; z: number }, blocked: DoorInfo[] = []): number | null {
    if (to.level !== HUB.level) return null;
    const path = findLongPath(HUB.level, HUB.x, HUB.z, to.x, to.z, 500, blocked);
    if (!path.length) return null;
    const end = path[path.length - 1];
    if (Math.max(Math.abs(end.x - to.x), Math.abs(end.z - to.z)) > 2) return null;
    let tiles = 0;
    let prev = { x: HUB.x, z: HUB.z };
    for (const step of path) {
        tiles += Math.max(Math.abs(step.x - prev.x), Math.abs(step.z - prev.z));
        prev = step;
    }
    return tiles;
}

for (const shop of shops) {
    const npcRequirement = npcGate(shop.npc);
    if (npcRequirement) {
        shop.accessible = false;
        shop.barrier = npcRequirement;
        shop.walkTiles = shop.level === 0 ? walkTiles(shop) : null;
        continue;
    }
    if (shop.level !== 0) {
        shop.accessible = false;
        shop.barrier = 'upstairs';
        shop.walkTiles = null;
        continue;
    }
    const open = walkTiles(shop);
    if (open === null) {
        shop.accessible = false;
        shop.barrier = 'unreachable';
        shop.walkTiles = null;
        continue;
    }
    const closed = walkTiles(shop, gatedDoors);
    if (closed === null) {
        // Reachable only through a gated door. Name the nearest gate's requirement.
        const nearest = gatedDoors
            .filter((d) => d.level === shop.level)
            .sort((a, b) => Math.hypot(a.x - shop.x, a.z - shop.z) - Math.hypot(b.x - shop.x, b.z - shop.z))[0];
        shop.accessible = false;
        shop.barrier = nearest ? `gated: ${nearest.requirement}` : 'gated';
        shop.walkTiles = open;
        continue;
    }
    shop.accessible = true;
    shop.barrier = null;
    shop.walkTiles = closed;
}

shops.sort((a, b) => b.buyMultiplier - a.buyMultiplier);

console.error(
    `${spawns.length} safe spawns worth 50+, ${clusters.length} clusters, ${shops.length} shops ` +
        `(${shops.filter((s) => s.accessible).length} reachable, ` +
        `${gatedDoors.length} gated doors from ${gatedLocIds.size} gated loc types)`,
);

if (process.argv.includes('--json')) {
    // The Rust side only needs the shops: it prices the exit, the game bot walks to it.
    console.log(JSON.stringify({ shops }, null, 0));
    process.exit(0);
}

console.log('// Generated by tools/build-money.ts — do not edit by hand.');
console.log(`export const DEFAULT_RESPAWN_TICKS = ${DEFAULT_RESPAWN_TICKS};\n`);
console.log('export interface SpawnItem { item: string; cost: number; x: number; z: number; respawnTicks: number }');
console.log('export interface SpawnCluster { level: number; x: number; z: number; value: number; items: SpawnItem[] }');
console.log('export interface Shop { npc: string; title: string; buyMultiplier: number; haggle: number; level: number; x: number; z: number; stock: Record<string, number>; buysAnything: boolean; safe: boolean; accessible: boolean; barrier: string | null; walkTiles: number | null }\n');
console.log(`export const SPAWN_CLUSTERS: SpawnCluster[] = ${JSON.stringify(clusters, null, 1)};\n`);
console.log(`export const SHOPS: Shop[] = ${JSON.stringify(shops, null, 1)};`);

/**
 * Work out which items the game can still produce, and which it cannot.
 *
 * This is what makes "capped supply" a measurable property rather than a vibe.
 * An item token's supply can only grow if a player can obtain the item in game
 * and bridge it out, so an item that nothing in the game world produces has a
 * supply fixed at whatever was seeded on chain.
 *
 * The test is a search for each item's debugname across the content scripts,
 * split by the kind of source it would indicate:
 *
 *   drop   — `scripts/drop tables/`, the per-NPC drop scripts
 *   shop   — any `.inv` shop stock list
 *   skill  — any `scripts/skill_*` directory (fishing, smithing, crafting, ...)
 *   quest  — any `scripts/quests/` reward
 *   spawn  — the `==== OBJ ====` ground-spawn sections of the map squares
 *
 * An item matching none of them has no route into a player's inventory, so its
 * on-chain supply cannot grow. That is a heuristic, not a proof — a name could
 * be produced by a script that spells it differently — so the output records
 * every signal rather than just the verdict, and the accumulator treats a high
 * bridged-in supply as evidence that overrides it.
 *
 *   bun bots/<name>/tools/build-item-sources.ts > crates/mercantile-core/data/item_sources.json
 */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = join(import.meta.dir, '..', '..', '..');
const CONTENT = join(ROOT, 'server', 'content');
const SCRIPTS = join(CONTENT, 'scripts');
const MAPS = join(CONTENT, 'maps');
const REGISTRY = join(ROOT, 'chain', 'registry', 'registry.json');

type Bucket = 'drop' | 'shop' | 'skill' | 'quest';

function walk(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir)) {
        const path = join(dir, entry);
        if (statSync(path).isDirectory()) walk(path, out);
        else out.push(path);
    }
    return out;
}

/** Classify a script path by the kind of source it would represent. */
function bucketOf(path: string): Bucket | null {
    // Unpacked reference dumps and test fixtures describe nothing the live world does.
    if (path.includes('/_unpack/') || path.includes('/_test/')) return null;
    if (path.includes('drop tables')) return 'drop';
    if (path.endsWith('.inv')) return 'shop';
    if (path.includes('/skill_')) return 'skill';
    if (path.includes('/quests/')) return 'quest';
    return null;
}

const corpus: Record<Bucket, string[]> = { drop: [], shop: [], skill: [], quest: [] };
for (const path of walk(SCRIPTS)) {
    const bucket = bucketOf(path);
    if (bucket) corpus[bucket].push(readFileSync(path, 'utf8'));
}
const joined = Object.fromEntries(
    Object.entries(corpus).map(([k, v]) => [k, v.join('\n')]),
) as Record<Bucket, string>;

// Ground spawns, by obj debugname.
const objNames = new Map<number, string>();
for (const line of readFileSync(join(CONTENT, 'pack', 'obj.pack'), 'utf8').split('\n')) {
    const m = line.match(/^(\d+)=(\S+)/);
    if (m?.[1] && m[2]) objNames.set(Number(m[1]), m[2]);
}
const spawned = new Set<string>();
for (const file of readdirSync(MAPS)) {
    if (!/^m\d+_\d+\.jm2$/.test(file)) continue;
    let section = '';
    for (const line of readFileSync(join(MAPS, file), 'utf8').split('\n')) {
        const t = line.trim();
        if (t.startsWith('====')) {
            section = t.replace(/=/g, '').trim();
            continue;
        }
        if (section !== 'OBJ') continue;
        const m = t.match(/^\d+\s+\d+\s+\d+:\s*(\d+)/);
        const name = m && objNames.get(Number(m[1]));
        if (name) spawned.add(name);
    }
}

const registry = JSON.parse(readFileSync(REGISTRY, 'utf8')) as {
    items: Record<string, unknown>;
};

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const sources: Record<string, Record<string, boolean>> = {};
let capped = 0;
for (const name of Object.keys(registry.items)) {
    const pattern = new RegExp(`\\b${escape(name)}\\b`);
    const signals = {
        drop: pattern.test(joined.drop),
        shop: pattern.test(joined.shop),
        skill: pattern.test(joined.skill),
        quest: pattern.test(joined.quest),
        spawn: spawned.has(name),
    };
    const isCapped = !Object.values(signals).some(Boolean);
    if (isCapped) capped++;
    sources[name] = { ...signals, capped: isCapped };
}

console.error(`${Object.keys(sources).length} items, ${capped} with no in-game source`);
console.log(JSON.stringify(sources, null, 0));

/**
 * The gazetteer.
 *
 * Every coordinate here was extracted from the game's own data, not recalled:
 *
 * * **Region labels** — `server/content/maps/labels.txt`, the map's own label list.
 * * **NPC positions** — the `==== NPC ====` sections of `server/content/maps/*.jm2`,
 *   where a spawn line `level localX localZ: npcId` becomes world
 *   `(mapX * 64 + localX, mapZ * 64 + localZ)`.
 * * **Teleport destinations** — `data=tele_coord` in
 *   `server/content/scripts/skill_magic/configs/magic_spells.dbrow`.
 * * **The manhole** — loc 881 in the map LOC sections; climbing down runs
 *   `p_telejump(movecoord(coord(), 0, 0, 6400))`, which is why the sewers are at
 *   surface level with `z + 6400` rather than a negative map level.
 *
 * Regenerate with `bun tools/build-places.ts` from a mercantile checkout when the
 * world data changes.
 */

import type { Place } from './types';
import { GENERATED_PLACES } from './places.generated';

/** How much z is added when descending into the sewers. */
export const SEWER_Z_OFFSET = 6400;

/**
 * Hand-curated points of interest, each verified individually against the source
 * named in its comment. Everything else — 136 region labels, 13 banks, 30 undead
 * sites, 16 chicken farms — comes from `places.generated.ts`.
 */
export const CURATED_PLACES: Place[] = [
    // ── Rune shops: NPC spawns from the map NPC sections ─────────────────────
    { id: 'aubury', name: "Aubury's rune shop", level: 0, x: 3253, z: 3402, tags: ['shop', 'runes', 'elemental_runes'] },
    { id: 'betty', name: "Betty's magic emporium", level: 0, x: 3012, z: 3259, tags: ['shop', 'runes', 'elemental_runes'] },

    // ── The Varrock sewers: loc 881 (manhole) from the map LOC sections.
    //    Climbing down runs p_telejump(movecoord(coord(), 0, 0, 6400)).
    { id: 'varrock_manhole', name: 'Varrock sewer manhole', level: 0, x: 3237, z: 3458, tags: ['transit'] },
    { id: 'varrock_sewers', name: 'Varrock sewers', level: 0, x: 3237, z: 3458 + SEWER_Z_OFFSET, tags: ['undead', 'dungeon'] },

    // ── The Wizards' Guild: doors are locs 1600/1601, the shopkeeper is
    //    magic_store_owner — who spawns on level 1, so the nature runes are
    //    upstairs. The door checks for 66 Magic (magic_guild.rs2).
    { id: 'wizards_guild_door', name: "Wizards' Guild door", level: 0, x: 2597, z: 3087, tags: ['transit', 'members'] },
    { id: 'wizards_guild', name: "Wizards' Guild ground floor", level: 0, x: 2594, z: 3089, tags: ['members'] },
    {
        id: 'wizards_guild_shop',
        name: "Wizards' Guild rune shop",
        level: 1,
        x: 2595,
        z: 3087,
        // The only unbounded nature rune supply in the game.
        tags: ['shop', 'runes', 'nature_runes', 'members'],
    },

    // ── Teleport landing spots: data=tele_coord in magic_spells.dbrow. These are
    //    where the bot actually arrives, which is what routing cares about.
    { id: 'tp_varrock', name: 'Varrock teleport arrival', level: 0, x: 3213, z: 3424, tags: ['teleport'] },
    { id: 'tp_lumbridge', name: 'Lumbridge teleport arrival', level: 0, x: 3221, z: 3218, tags: ['teleport'] },
    { id: 'tp_falador', name: 'Falador teleport arrival', level: 0, x: 2965, z: 3378, tags: ['teleport'] },
    { id: 'tp_camelot', name: 'Camelot teleport arrival', level: 0, x: 2757, z: 3478, tags: ['teleport'] },
    { id: 'tp_ardougne', name: 'Ardougne teleport arrival', level: 0, x: 2661, z: 3301, tags: ['teleport'] },
];

/** Curated points of interest plus everything extracted from the map data. */
export const PLACES: Place[] = [
    ...CURATED_PLACES,
    // Generated ids never shadow a curated one.
    ...GENERATED_PLACES.filter(
        (generated) => !CURATED_PLACES.some((curated) => curated.id === generated.id),
    ),
];

const BY_ID = new Map(PLACES.map((place) => [place.id, place]));

/** Look a place up by id. */
export function place(id: string): Place {
    const found = BY_ID.get(id);
    if (!found) throw new Error(`unknown place: ${id}`);
    return found;
}

/** Every place carrying a tag. */
export function placesTagged(tag: string): Place[] {
    return PLACES.filter((p) => p.tags.includes(tag));
}

/** Chebyshev distance in tiles — how RuneScape measures movement. */
export function tileDistance(a: { x: number; z: number }, b: { x: number; z: number }): number {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
}

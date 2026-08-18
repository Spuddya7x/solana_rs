/**
 * The transitions a walk cannot make.
 *
 * Walking is handled by the pathfinder and needs no edges here — the router asks
 * the collision map directly. This file holds only what walking *cannot* do:
 * teleports, ladders and manholes, boats.
 *
 * Teleport data (level, runes, destination) comes from
 * `skill_magic/configs/magic_spells.dbrow`; component ids are positional in the
 * magic interface, the same derivation as `lib/spells.ts`.
 */

import type { Link } from './types';

/** Teleport spells, as one-way links from anywhere to their destination. */
export const TELEPORTS: Link[] = [
    {
        from: '*',
        to: 'tp_varrock',
        kind: 'teleport',
        // A teleport is near-instant next to a cross-region walk, but not free:
        // the cast, the animation and the arrival cost a few ticks.
        cost: 8,
        spellComponent: 1164,
        requires: { magicLevel: 25, runes: { firerune: 1, airrune: 3, lawrune: 1 } },
        note: 'Varrock teleport',
    },
    {
        from: '*',
        to: 'tp_lumbridge',
        kind: 'teleport',
        cost: 8,
        spellComponent: 1167,
        requires: { magicLevel: 31, runes: { earthrune: 1, airrune: 3, lawrune: 1 } },
        note: 'Lumbridge teleport',
    },
    {
        from: '*',
        to: 'tp_falador',
        kind: 'teleport',
        cost: 8,
        spellComponent: 1170,
        requires: { magicLevel: 37, runes: { waterrune: 1, airrune: 3, lawrune: 1 } },
        note: 'Falador teleport',
    },
    {
        from: '*',
        to: 'tp_camelot',
        kind: 'teleport',
        cost: 8,
        spellComponent: 1174,
        requires: { magicLevel: 45, runes: { airrune: 5, lawrune: 1 } },
        note: 'Camelot teleport',
    },
    {
        from: '*',
        to: 'tp_ardougne',
        kind: 'teleport',
        cost: 8,
        spellComponent: 1540,
        requires: { magicLevel: 51, runes: { waterrune: 2, lawrune: 2 } },
        note: 'Ardougne teleport (needs Plague City completed)',
    },
];

/** Scenery and NPC transitions. */
export const TRANSITIONS: Link[] = [
    {
        from: 'varrock_manhole',
        to: 'varrock_sewers',
        kind: 'object',
        // Open the cover, climb down: two interactions and a short delay.
        cost: 6,
        object: { name: /manhole/i, option: 1 },
        note: 'climb down the manhole into the sewers',
    },
    {
        from: 'varrock_sewers',
        to: 'varrock_manhole',
        kind: 'object',
        cost: 6,
        object: { name: /ladder/i, option: 1 },
        note: 'climb the ladder back to the surface',
    },
    {
        from: 'wizards_guild_door',
        to: 'wizards_guild',
        kind: 'object',
        cost: 4,
        object: { name: /door/i, option: 1 },
        // magic_guild.rs2 turns you away below 66: "You need a magic level of 66."
        requires: { magicLevel: 66 },
        note: 'through the guild door (66 Magic)',
    },
    {
        from: 'wizards_guild',
        to: 'wizards_guild_shop',
        kind: 'object',
        cost: 6,
        object: { name: /staircase|stairs/i, option: 'Climb-up' },
        note: 'upstairs to the rune shop',
    },
];

/** Every link the router knows about. */
export const LINKS: Link[] = [...TELEPORTS, ...TRANSITIONS];

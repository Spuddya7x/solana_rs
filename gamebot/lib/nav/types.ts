/**
 * Navigation types.
 *
 * The SDK already solves *tile-level* movement: `bot.walkTo` pathfinds over the
 * full collision map, opens doors, and detects being stuck. What it cannot do is
 * anything that is not a walk — climb a ladder, take a boat, cast a teleport —
 * or answer "where is the nearest bank". This layer sits on top and supplies
 * exactly that: a gazetteer of named places, a graph of the non-walking
 * transitions between them, and a router that mixes the two.
 */

/** A named location in the world. */
export interface Place {
    /** Stable identifier used in routes and scripts. */
    id: string;
    name: string;
    /** Map level: 0 is the surface, 1 and 2 are upstairs, and the sewers sit at
     *  surface level with `z + 6400` rather than a negative level. */
    level: number;
    x: number;
    z: number;
    /** Free-form labels for "nearest bank", "somewhere with undead", and so on. */
    tags: string[];
}

/** What the bot can currently do, which decides the routes open to it. */
export interface Capabilities {
    magicLevel: number;
    /** Count of an item by name, e.g. `runes('lawrune')`. */
    runes: (name: string) => number;
    /** Coins carried. */
    gp: number;
}

/** A requirement a link imposes before it can be taken. */
export interface Requirement {
    magicLevel?: number;
    /** Runes consumed, by item name. */
    runes?: Record<string, number>;
    gp?: number;
}

/** How a link is traversed. */
export type LinkKind =
    /** Ordinary walking; cost comes from the pathfinder. */
    | 'walk'
    /** A spell cast from the magic tab. */
    | 'teleport'
    /** Interacting with a scenery object: ladder, staircase, manhole, gate. */
    | 'object'
    /** Talking to an NPC, e.g. a boat operator. */
    | 'npc';

/** A non-walking transition between two places. */
export interface Link {
    from: string;
    to: string;
    kind: Exclude<LinkKind, 'walk'>;
    /** Rough cost in game ticks, used to compare routes. */
    cost: number;
    requires?: Requirement;
    /** For `object` links: the scenery to interact with, and which option. */
    object?: { name: string | RegExp; option?: number | string };
    /** For `npc` links: who to talk to, and the dialogue path to follow. */
    npc?: { name: string | RegExp; dialog?: (string | RegExp | number)[] };
    /** For `teleport` links: the magic interface component to click. */
    spellComponent?: number;
    /** Shown in logs so a route reads like instructions. */
    note?: string;
}

/** One leg of a planned route. */
export interface Step {
    kind: LinkKind;
    to: Place;
    cost: number;
    link?: Link;
}

/** A planned route. */
export interface Route {
    steps: Step[];
    /** Total estimated cost in ticks. */
    cost: number;
}

/** Seconds per game tick, for turning costs into wall-clock estimates. */
export const TICK_SECONDS = 0.6;

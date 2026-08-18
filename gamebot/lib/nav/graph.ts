/**
 * The router.
 *
 * Walking is not modelled as edges — asking the collision map is both cheaper to
 * maintain and more accurate than a hand-drawn road network, and the SDK already
 * ships the full map. So the graph holds only the transitions a walk cannot make
 * (teleports, ladders, doors with requirements) and asks a `walkCost` function
 * for everything else.
 *
 * That function is injected rather than imported so the router can be tested
 * without loading the pathfinder, and so a caller can swap in a cheaper estimate
 * when planning in bulk.
 */

import { LINKS } from './links';
import { PLACES, tileDistance } from './places';
import type { Capabilities, Link, Place, Requirement, Route, Step } from './types';

/** Answers "how many ticks to walk from here to there", or null if unreachable. */
export type WalkCost = (from: Place, to: Place) => number | null;

/** A straight-line estimate, for planning without the collision map. */
export const estimateWalkCost: WalkCost = (from, to) => {
    // Different levels always need a stairs or ladder link, never a walk.
    if (from.level !== to.level) return null;
    // Roughly one tile per tick walking; running halves it but is not assumed.
    return tileDistance(from, to);
};

/** Whether the bot currently meets a requirement. */
export function meets(requirement: Requirement | undefined, caps: Capabilities): boolean {
    if (!requirement) return true;
    if (requirement.magicLevel !== undefined && caps.magicLevel < requirement.magicLevel) {
        return false;
    }
    if (requirement.gp !== undefined && caps.gp < requirement.gp) return false;
    for (const [rune, count] of Object.entries(requirement.runes ?? {})) {
        if (caps.runes(rune) < count) return false;
    }
    return true;
}

/** Everything the bot can do that is not walking. */
export class NavGraph {
    private readonly places = new Map<string, Place>();
    private readonly links: Link[];

    constructor(places: Place[] = PLACES, links: Link[] = LINKS) {
        for (const place of places) this.places.set(place.id, place);
        this.links = links;
    }

    place(id: string): Place {
        const found = this.places.get(id);
        if (!found) throw new Error(`unknown place: ${id}`);
        return found;
    }

    all(): Place[] {
        return [...this.places.values()];
    }

    tagged(tag: string): Place[] {
        return this.all().filter((place) => place.tags.includes(tag));
    }

    /** Links leaving a place, including the ones available from anywhere (`'*'`). */
    linksFrom(id: string, caps: Capabilities): Link[] {
        return this.links.filter(
            (link) =>
                (link.from === id || link.from === '*') &&
                link.to !== id &&
                this.places.has(link.to) &&
                meets(link.requires, caps),
        );
    }

    /**
     * Cheapest route from a position to a place, in ticks.
     *
     * Dijkstra over places, where every pair is connected by a walk if the
     * pathfinder says so, plus the explicit links. Returns `null` when the
     * destination cannot be reached with the bot's current capabilities — a
     * teleport it has no runes for is simply not an edge.
     */
    route(
        from: { x: number; z: number; level: number },
        toId: string,
        caps: Capabilities,
        walkCost: WalkCost = estimateWalkCost,
    ): Route | null {
        const start: Place = { id: '@start', name: 'here', tags: [], ...from };
        const target = this.place(toId);
        // Standing on it is not a journey. Without this the router emits a
        // zero-cost walk step, which reads as a move that never happens.
        if (start.level === target.level && tileDistance(start, target) === 0) {
            return { steps: [], cost: 0 };
        }

        const nodes = new Map<string, Place>([[start.id, start], ...this.places]);
        const dist = new Map<string, number>([[start.id, 0]]);
        const prev = new Map<string, { from: string; step: Step }>();
        const settled = new Set<string>();

        while (settled.size < nodes.size) {
            // Smallest unsettled distance. The graph is small enough that a
            // linear scan beats the complexity of a heap.
            let currentId: string | null = null;
            let best = Number.POSITIVE_INFINITY;
            for (const [id, d] of dist) {
                if (!settled.has(id) && d < best) {
                    best = d;
                    currentId = id;
                }
            }
            if (currentId === null) break;
            if (currentId === target.id) break;
            settled.add(currentId);
            const current = nodes.get(currentId)!;

            const relax = (next: Place, cost: number, step: Step) => {
                const through = best + cost;
                if (through < (dist.get(next.id) ?? Number.POSITIVE_INFINITY)) {
                    dist.set(next.id, through);
                    prev.set(next.id, { from: currentId!, step: { ...step, cost } });
                }
            };

            for (const link of this.linksFrom(currentId, caps)) {
                const next = this.place(link.to);
                relax(next, link.cost, { kind: link.kind, to: next, cost: link.cost, link });
            }
            for (const next of this.places.values()) {
                if (next.id === currentId || settled.has(next.id)) continue;
                const cost = walkCost(current, next);
                if (cost === null) continue;
                relax(next, cost, { kind: 'walk', to: next, cost });
            }
        }

        if (!dist.has(target.id) || !Number.isFinite(dist.get(target.id)!)) return null;

        const steps: Step[] = [];
        let cursor = target.id;
        while (cursor !== start.id) {
            const entry = prev.get(cursor);
            if (!entry) return null;
            steps.unshift(entry.step);
            cursor = entry.from;
        }
        return { steps, cost: dist.get(target.id)! };
    }

    /** The cheapest place carrying `tag`, and the route to it. */
    nearestTagged(
        from: { x: number; z: number; level: number },
        tag: string,
        caps: Capabilities,
        walkCost: WalkCost = estimateWalkCost,
    ): { place: Place; route: Route } | null {
        let best: { place: Place; route: Route } | null = null;
        for (const candidate of this.tagged(tag)) {
            const route = this.route(from, candidate.id, caps, walkCost);
            if (route && (best === null || route.cost < best.route.cost)) {
                best = { place: candidate, route };
            }
        }
        return best;
    }
}

/** A graph over the shipped gazetteer and links. */
export const NAV = new NavGraph();

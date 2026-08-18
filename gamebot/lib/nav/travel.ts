/**
 * Executing a route.
 *
 * Each step is either a walk — handed to the SDK, which pathfinds and opens
 * doors — or a transition the SDK has no concept of. Arrival is verified after
 * every step rather than assumed, because a failed climb that goes unnoticed
 * leaves the bot casting spells at the wrong floor.
 */

import type { BotActions } from '../../../../sdk/actions';
import type { BotSDK } from '../../../../sdk/index';
import { NAV, NavGraph, type WalkCost } from './graph';
import { tileDistance } from './places';
import type { Capabilities, Place, Route, Step } from './types';

export interface TravelResult {
    success: boolean;
    message: string;
    /** Steps completed before stopping. */
    completed: number;
    /** Where the bot ended up. */
    at?: { x: number; z: number; level: number };
}

export interface TravelOptions {
    graph?: NavGraph;
    walkCost?: WalkCost;
    /** How close counts as arrived. */
    tolerance?: number;
    /** Attempts per step before giving up. */
    retries?: number;
}

/** Read the bot's current capabilities out of the world state. */
export function capabilities(sdk: BotSDK): Capabilities {
    return {
        magicLevel: sdk.getSkill('magic')?.level ?? 1,
        runes: (name: string) => {
            // "lawrune" -> /^law rune$/i, matching how items are named in game.
            const spaced = name.replace(/rune$/, ' rune');
            return sdk.countInventoryItems(new RegExp(`^${spaced}$`, 'i'));
        },
        gp: sdk.countInventoryItems(/^coins$/i),
    };
}

/** Where the bot is now. */
export function here(sdk: BotSDK): { x: number; z: number; level: number } {
    const player = sdk.getState()?.player;
    if (!player) throw new Error('no player state — is the bot in game?');
    return { x: player.worldX, z: player.worldZ, level: player.level ?? 0 };
}

/** Plan and walk a route to a named place. */
export async function travelTo(
    bot: BotActions,
    sdk: BotSDK,
    destination: string,
    options: TravelOptions = {},
): Promise<TravelResult> {
    const graph = options.graph ?? NAV;
    const tolerance = options.tolerance ?? 3;
    const retries = options.retries ?? 2;
    const caps = capabilities(sdk);
    const start = here(sdk);

    const target = graph.place(destination);
    if (start.level === target.level && tileDistance(start, target) <= tolerance) {
        return { success: true, message: `already at ${target.name}`, completed: 0, at: start };
    }

    const route = graph.route(start, destination, caps, options.walkCost);
    if (!route) {
        return {
            success: false,
            message:
                `no route to ${target.name} with the current kit ` +
                `(magic ${caps.magicLevel}, ${caps.runes('lawrune')} law runes)`,
            completed: 0,
            at: start,
        };
    }

    console.log(
        `route to ${target.name}: ${route.steps.length} steps, about ` +
            `${Math.round(route.cost * 0.6)}s — ${describe(route)}`,
    );

    let completed = 0;
    for (const step of route.steps) {
        let done = false;
        for (let attempt = 0; attempt <= retries && !done; attempt++) {
            done = await runStep(bot, sdk, step, tolerance);
            if (!done && attempt < retries) {
                console.log(`  retrying ${label(step)}`);
                await bot.dismissBlockingUI();
            }
        }
        if (!done) {
            return {
                success: false,
                message: `stuck at ${label(step)}`,
                completed,
                at: here(sdk),
            };
        }
        completed += 1;
    }

    const finished = here(sdk);
    const arrived = finished.level === target.level && tileDistance(finished, target) <= tolerance + 5;
    return {
        success: arrived,
        message: arrived ? `arrived at ${target.name}` : `route finished but ended up off target`,
        completed,
        at: finished,
    };
}

async function runStep(
    bot: BotActions,
    sdk: BotSDK,
    step: Step,
    tolerance: number,
): Promise<boolean> {
    switch (step.kind) {
        case 'walk': {
            const result = await bot.walkTo(step.to.x, step.to.z, tolerance);
            return result.success && atPlace(sdk, step.to, tolerance + 5);
        }
        case 'teleport': {
            const component = step.link?.spellComponent;
            if (component === undefined) return false;
            const before = here(sdk);
            const clicked = await sdk.sendClickComponent(component);
            if (!clicked.success) return false;
            // A teleport lands within a couple of tiles of its destination; wait
            // for the position to actually change rather than trusting the click.
            await sdk.waitForCondition(
                (state) =>
                    !!state.player &&
                    (Math.abs(state.player.worldX - before.x) > 8 ||
                        Math.abs(state.player.worldZ - before.z) > 8),
                10_000,
            ).catch(() => undefined);
            return atPlace(sdk, step.to, 12);
        }
        case 'object': {
            const spec = step.link?.object;
            if (!spec) return false;
            const result = await bot.interactLoc(spec.name, spec.option ?? 1);
            if (!result.success) return false;
            await sdk.waitForCondition(
                (state) => !!state.player && nearPlace(state.player, step.to, 24),
                10_000,
            ).catch(() => undefined);
            return atPlace(sdk, step.to, 24);
        }
        case 'npc': {
            const spec = step.link?.npc;
            if (!spec) return false;
            const talked = await bot.talkTo(spec.name);
            if (!talked.success) return false;
            if (spec.dialog) await bot.navigateDialog(spec.dialog);
            await bot.waitForDialogClose(15_000);
            return atPlace(sdk, step.to, 24);
        }
    }
}

function nearPlace(
    player: { worldX: number; worldZ: number; level?: number },
    place: Place,
    tolerance: number,
): boolean {
    return (
        (player.level ?? 0) === place.level &&
        tileDistance({ x: player.worldX, z: player.worldZ }, place) <= tolerance
    );
}

function atPlace(sdk: BotSDK, place: Place, tolerance: number): boolean {
    const player = sdk.getState()?.player;
    return player ? nearPlace(player, place, tolerance) : false;
}

function label(step: Step): string {
    return step.link?.note ?? `${step.kind} to ${step.to.name}`;
}

function describe(route: Route): string {
    return route.steps.map(label).join(' -> ');
}

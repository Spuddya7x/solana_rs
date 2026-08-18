/**
 * The ground-spawn circuit — a fresh account's only real income.
 *
 * Free items lie on the floor and respawn every 100 ticks (a minute) by default.
 * No levels, no combat, no runes. Two clusters near Varrock carry almost all of
 * the safe value:
 *
 * * **Varrock sewers** — ruby ring, gold necklace, gold bar, gold ore: about
 *   1,755 GP of shop value in one stop, down the manhole the nav graph knows.
 * * **Varrock surface** — iron platebody, iron platelegs, iron sword: about 558
 *   GP, and the first two *are* the splash kit, so the opening loop also equips
 *   the account for training.
 *
 * Everything else worth 50 GP or more is in the wilderness, where an unattended
 * bot is someone else's loot.
 */

import type { BotActions } from '../../../../sdk/actions';
import type { BotSDK } from '../../../../sdk/index';
import { SPAWN_CLUSTERS, type SpawnCluster } from './world.generated';

export interface LootRunResult {
    picked: number;
    value: number;
    message: string;
}

/** Clusters worth visiting, richest first. */
export function bestClusters(limit = 3): SpawnCluster[] {
    return SPAWN_CLUSTERS.slice(0, limit);
}

/**
 * Walk one cluster and pick up everything in it.
 *
 * Items are collected by name from the ground rather than by coordinate: a
 * respawned item can sit a tile off, and the SDK's pickup already walks to it.
 */
export async function collectCluster(
    bot: BotActions,
    sdk: BotSDK,
    cluster: SpawnCluster,
): Promise<LootRunResult> {
    let picked = 0;
    let value = 0;
    for (const spawn of cluster.items) {
        const before = sdk.countInventoryItems(itemPattern(spawn.item));
        const result = await bot.pickupItem(itemPattern(spawn.item));
        if (!result.success) continue;
        if (sdk.countInventoryItems(itemPattern(spawn.item)) > before) {
            picked += 1;
            value += spawn.cost;
        }
    }
    return {
        picked,
        value,
        message: picked
            ? `picked ${picked} items worth ${value} gp of cost`
            : 'nothing on the ground — respawns take about a minute',
    };
}

/** "iron_platebody" -> /^iron platebody$/i, matching the in-game item name. */
export function itemPattern(item: string): RegExp {
    return new RegExp(`^${item.replace(/_/g, ' ')}$`, 'i');
}

/** Seconds until a cluster is worth revisiting. */
export function respawnSeconds(cluster: SpawnCluster): number {
    return Math.max(...cluster.items.map((i) => i.respawnTicks)) * 0.6;
}

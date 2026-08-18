/**
 * Funding a fresh account.
 *
 * Training to the level alchemy needs costs about 113,000 GP of runes, and a new
 * character has about 25. This closes that gap, and the route it takes was
 * chosen by measuring the world rather than by intuition:
 *
 * * **Cow hides are worth nothing here.** `cow_hide` has `cost = 1`, and shops
 *   pay `cost x multiplier / 1000` with multipliers of 600–700, so a hide sells
 *   for 0 GP. Beef, bones, feathers and raw chicken are all `cost = 1` too. The
 *   classic hide run does not work in this economy.
 * * **Low-level monster drops are worth single digits.** Reading the drop tables'
 *   own `random(128)` thresholds, a barbarian averages about 11 GP a kill and a
 *   cow or chicken rather less. The valuable tables all belong to monsters a
 *   fresh account cannot fight.
 * * **Ground spawns pay properly.** Two clusters near Varrock hold about 2,300 GP
 *   of shop value and respawn every minute, for no levels and no combat. The
 *   surface one is the splash kit, so the first loop also equips the account.
 *
 * Usage:
 *   bun bots/<name>/moneymaker.ts --target 20000
 *   bun bots/<name>/moneymaker.ts --loops 5 --no-sell
 */

import { runScript } from '../../sdk/runner';
import { NAV, capabilities, here, travelTo } from './lib/nav';
import { bestClusters, collectCluster, itemPattern, respawnSeconds } from './lib/money/loot-run';
import { sellPriceFor, willBuy, worthwhileCount } from './lib/money/pricing';
import { SHOPS, type Shop } from './lib/money/world.generated';

const args = process.argv.slice(2);
const flag = (name: string, fallback: string) => {
    const i = args.indexOf(`--${name}`);
    return i === -1 ? fallback : (args[i + 1] ?? fallback);
};

/** Stop once the character is carrying this much GP. */
const TARGET_GP = Number(flag('target', '20000'));
/** Hard cap on circuits, so an unattended run is bounded. */
const MAX_LOOPS = Number(flag('loops', '40'));
const SELL = !args.includes('--no-sell');

await runScript(async ({ bot, sdk }) => {
    await sdk.waitForReady();
    const clusters = bestClusters(Number(flag('clusters', '2')));
    const start = sdk.countInventoryItems(/^coins$/i);
    console.log(
        `funding run: ${start} gp now, target ${TARGET_GP}. Circuit: ` +
            clusters.map((c) => `(${c.x},${c.z}) ~${c.value} gp`).join(' -> '),
    );

    let loops = 0;
    let collected = 0;
    for (; loops < MAX_LOOPS; loops++) {
        const coins = sdk.countInventoryItems(/^coins$/i);
        if (coins >= TARGET_GP) break;

        let pickedThisLoop = 0;
        for (const cluster of clusters) {
            // Clusters are keyed by coordinate rather than by a gazetteer id, so
            // route to the nearest known place and walk the last stretch.
            const near = NAV.all()
                .filter((p) => p.level === cluster.level)
                .sort(
                    (a, b) =>
                        Math.hypot(a.x - cluster.x, a.z - cluster.z) -
                        Math.hypot(b.x - cluster.x, b.z - cluster.z),
                )[0];
            if (near) await travelTo(bot, sdk, near.id);
            await bot.walkTo(cluster.x, cluster.z, 2);

            const result = await collectCluster(bot, sdk, cluster);
            pickedThisLoop += result.picked;
            collected += result.picked;
            console.log(`  [${loops + 1}] (${cluster.x},${cluster.z}): ${result.message}`);
        }

        if (SELL && pickedThisLoop > 0) {
            const sold = await sellLoot();
            console.log(`  sold: ${sold}`);
        }

        if (pickedThisLoop === 0) {
            // Everything was still on respawn; waiting beats walking the loop dry.
            const wait = Math.max(...clusters.map(respawnSeconds));
            console.log(`  nothing to pick up — waiting ${wait.toFixed(0)}s for respawns`);
            await new Promise((resolve) => setTimeout(resolve, wait * 1000));
        }
    }

    const finish = sdk.countInventoryItems(/^coins$/i);
    console.log(`\n${loops} loops, ${collected} items, ${finish - start} gp earned (${finish} total)`);
    return { loops, collected, gpEarned: finish - start, gp: finish };

    /** Sell everything picked up at the best shop that will take it. */
    async function sellLoot(): Promise<string> {
        const loot = new Set(clusters.flatMap((c) => c.items.map((i) => i.item)));
        const held = [...loot].filter((item) => sdk.findInventoryItem(itemPattern(item)));
        if (held.length === 0) return 'nothing to sell';

        // One shop rarely takes everything, and the price decays as you sell, so
        // pick the shop that pays most for the biggest slice of the haul.
        const shop = pickShop(held);
        if (!shop) return 'no shop will buy this';
        const place = NAV.all()
            .filter((p) => p.level === shop.level)
            .sort(
                (a, b) => Math.hypot(a.x - shop.x, a.z - shop.z) - Math.hypot(b.x - shop.x, b.z - shop.z),
            )[0];
        if (place) await travelTo(bot, sdk, place.id);
        await bot.walkTo(shop.x, shop.z, 3);
        if (!(await bot.openShop()).success) return `could not open ${shop.title}`;

        const before = sdk.countInventoryItems(/^coins$/i);
        for (const item of held) {
            if (!willBuy(shop, item)) continue;
            const cost = costOf(item);
            // Stop when the marginal price falls under a third of cost; the next
            // shop, or the next respawn, is worth more than the last few units.
            const worth = worthwhileCount(shop, item, cost, Math.max(1, Math.floor(cost / 3)));
            const have = sdk.countInventoryItems(itemPattern(item));
            const sell = Math.min(worth, have);
            if (sell > 0) await bot.sellToShop(itemPattern(item), sell);
        }
        await bot.closeShop();
        return `${sdk.countInventoryItems(/^coins$/i) - before} gp at ${shop.title}`;
    }

    function costOf(item: string): number {
        for (const cluster of clusters) {
            const found = cluster.items.find((i) => i.item === item);
            if (found) return found.cost;
        }
        return 1;
    }

    /** The shop paying most for this haul, nearest first among equals. */
    function pickShop(items: string[]): Shop | undefined {
        const position = here(sdk);
        return SHOPS.map((shop) => {
            const value = items
                .filter((item) => willBuy(shop, item))
                .reduce((sum, item) => sum + sellPriceFor(shop, item, costOf(item), 0), 0);
            const distance = Math.hypot(shop.x - position.x, shop.z - position.z);
            return { shop, value, distance };
        })
            .filter((c) => c.value > 0)
            .sort((a, b) => b.value - a.value || a.distance - b.distance)[0]?.shop;
    }
});

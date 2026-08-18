/**
 * Picking pockets in Lumbridge, and paying the hitpoint bill it runs up.
 *
 * The fastest money a brand-new account can make. A `man3` stands three tiles
 * from where the tutorial drops you, his pocket always holds exactly three
 * coins, and seven attempts in ten succeed at Thieving 1 — no capital, no
 * levels, no equipment, no walking.
 *
 * The catch is the other three in ten. `~fail_pick_pocket` stuns for eight ticks
 * and takes a hitpoint, which at this cadence is **434 damage an hour** against
 * sixty of passive regeneration. Left alone the account dies in about four
 * minutes, and `~damage_self` queues `player_death` the moment hitpoints reach
 * zero — dropping the coins the run exists to collect. So the healing is not a
 * nicety bolted on the side; it is most of the program.
 *
 * Two ways to pay it, chosen with `--recover`:
 *
 * * `forage` (default) — walk to Lumbridge Swamp, net shrimp with the tutorial's
 *   free net, chop and light a fire at the tree that sits exactly on the line
 *   home, cook, and carry the stack back. Slow but self-funding. See `lib/forage`.
 * * `idle` — stop thieving and do nothing until hitpoints come back at one a
 *   minute. Costs nothing, wastes an enormous amount of clock: recovering the
 *   damage from one minute of thieving takes seven minutes of standing still.
 *   Useful when the account has no tools, or for a short supervised run.
 *
 * `mercbot thieve-plan` costs both, and the ladder it prints is worth reading
 * before starting a long run: Thieving 10 (about seven minutes of men) doubles
 * the account's income by unlocking farmers, and is the single biggest upgrade
 * available.
 *
 * Usage:
 *   bun bots/<name>/thief.ts --target 5000
 *   bun bots/<name>/thief.ts --recover idle --minutes 20
 */

import { runScript } from '../../sdk/runner';
import { forage, hasTools, missingTools, SHRIMP_HEAL } from './lib/forage';
import {
    bestTarget,
    canContinue,
    damagePerHour,
    healingNeeded,
    hitpointFloor,
    hitpoints,
    successChance,
    type Target,
} from './lib/thieving';

const args = process.argv.slice(2);
const flag = (name: string, fallback: string) => {
    const i = args.indexOf(`--${name}`);
    return i === -1 ? fallback : (args[i + 1] ?? fallback);
};

/** Stop once the account is carrying this much GP. */
const TARGET_GP = Number(flag('target', '5000'));
/** Hard wall-clock cap, so an unattended run is bounded. */
const MAX_MINUTES = Number(flag('minutes', '120'));
/** How to get hitpoints back: `forage` or `idle`. */
const RECOVER = flag('recover', 'forage') as 'forage' | 'idle';
/** Minutes of thieving one forage trip should fund. */
const TRIP_MINUTES = Number(flag('trip', '12'));

await runScript(async ({ bot, sdk }) => {
    await sdk.waitForReady();

    const thieving = sdk.getSkill('thieving')?.level ?? 1;
    const target = bestTarget(thieving);
    const hp = hitpoints(sdk);
    const start = sdk.countInventoryItems(/^coins$/i);

    console.log(
        `thieving: ${target.name} at (${target.where.x},${target.where.z}), ` +
            `Thieving ${thieving} -> ${(successChance(target, thieving) * 100).toFixed(0)}% success, ` +
            `${damagePerHour(target, thieving).toFixed(0)} damage/hour`,
    );
    console.log(
        `recovery: ${RECOVER}. ${start} gp now, target ${TARGET_GP}, floor ${hitpointFloor(target)} hp` +
            (hp ? ` (at ${hp.current}/${hp.max})` : ''),
    );
    if (RECOVER === 'forage' && !hasTools(sdk)) {
        console.log(
            `warning: missing ${missingTools(sdk).join(', ')} — the forage loop needs all three ` +
                '(the tutorial grants them). Falling back to idle recovery.',
        );
    }

    const deadline = Date.now() + MAX_MINUTES * 60_000;
    let attempts = 0;
    let successes = 0;
    let trips = 0;

    while (Date.now() < deadline) {
        if (sdk.countInventoryItems(/^coins$/i) >= TARGET_GP) break;

        // --- eat back into the green before deciding anything else ---
        while (!canContinue(sdk, target) && sdk.countInventoryItems(/^shrimps$/i) > 0) {
            await bot.eatFood(/^shrimps$/i);
            await sdk.waitForTicks(3);
        }

        if (!canContinue(sdk, target)) {
            const current = hitpoints(sdk);
            if (RECOVER === 'forage' && hasTools(sdk)) {
                const wanted = healingNeeded(target, thieving, TRIP_MINUTES);
                console.log(
                    `hp ${current?.current}/${current?.max}: out of food, foraging for ${wanted.toFixed(0)} hp`,
                );
                const result = await forage(sdk, bot, wanted, { log: (m) => console.log(m) });
                trips++;
                if (result.cooked === 0) {
                    console.log(`forage failed (${result.reason}); idling instead`);
                    await idleUntilSafe(sdk, target);
                }
                continue;
            }
            console.log(`hp ${current?.current}/${current?.max}: idling to regenerate`);
            await idleUntilSafe(sdk, target);
            continue;
        }

        // --- pick a pocket ---
        const npc = sdk.findNearbyNpc(target.pattern);
        if (!npc) {
            await bot.walkTo(target.where.x, target.where.z);
            await sdk.waitForTicks(3);
            continue;
        }

        const before = sdk.countInventoryItems(/^coins$/i);
        await bot.interactNpc(npc, /pickpocket/i);
        attempts++;
        // A success resolves next tick; a failure holds `%action_delay` for the
        // full stun, so waiting it out here is what keeps the send rate honest.
        await sdk.waitForTicks(2);
        if (sdk.countInventoryItems(/^coins$/i) > before) {
            successes++;
        } else {
            await sdk.waitForTicks(target.stunTicks);
        }

        if (attempts % 50 === 0) {
            const coins = sdk.countInventoryItems(/^coins$/i);
            const current = hitpoints(sdk);
            console.log(
                `${attempts} attempts, ${((successes / attempts) * 100).toFixed(0)}% success, ` +
                    `${coins - start} gp, ${current?.current}/${current?.max} hp, ` +
                    `${sdk.countInventoryItems(/^shrimps$/i)} shrimp left`,
            );
        }
    }

    const earned = sdk.countInventoryItems(/^coins$/i) - start;
    const minutes = (MAX_MINUTES * 60_000 - (deadline - Date.now())) / 60_000;
    console.log(
        `done: ${earned} gp in ${minutes.toFixed(1)} min over ${attempts} attempts ` +
            `(${attempts ? ((successes / attempts) * 100).toFixed(0) : 0}% success, ${trips} forage trips) ` +
            `= ${minutes > 0 ? (earned / (minutes / 60)).toFixed(0) : 0} gp/hour`,
    );
});

/**
 * Stand still until hitpoints clear the floor again.
 *
 * One hitpoint per hundred ticks, so this is genuinely a minute a point. It is
 * bounded so a stuck account gives up rather than idling forever.
 */
async function idleUntilSafe(
    sdk: Parameters<typeof hitpoints>[0],
    target: Target,
    maxTicks = 3_000,
): Promise<void> {
    const floor = hitpointFloor(target);
    for (let waited = 0; waited < maxTicks; waited += 100) {
        const hp = hitpoints(sdk);
        // Recover a little past the floor, or the very next failure stops us again.
        if (hp && hp.current >= Math.min(hp.max, floor + SHRIMP_HEAL * 2)) return;
        await sdk.waitForTicks(100);
    }
}

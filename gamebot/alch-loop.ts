/**
 * The production loop: turn bridged-in items into GP.
 *
 * One pass is: claim whatever the chain side deposited, alch every target item
 * in the inventory, then hand the GP back to the Exchange Clerk for the wallet.
 * It is deliberately a single pass rather than an endless loop — the item supply
 * comes from `mercbot` buying pools out on the other side, so the natural cadence
 * is "run this after a buy", and a script that exits makes that easy to schedule.
 *
 * Requires level 55 Magic, a staff of fire (removes five runes a cast, forever)
 * and nature runes — which are themselves a tokenised item, so the chain side can
 * supply them at their own pool floor of about 7 GP.
 *
 * Usage:
 *   bun bots/<name>/alch-loop.ts --alch "rune platebody,dragon med helm" --withdraw-gp
 */

import { runScript } from '../../sdk/runner';
import { alchInventory, missingRunes } from './lib/alch';
import { SPELLS } from './lib/spells';
import { claimDeposits, withdrawGpToWallet } from './lib/bridge';
import { NAV, capabilities, here, travelTo } from './lib/nav';

const args = process.argv.slice(2);
const flag = (name: string, fallback: string) => {
    const index = args.indexOf(`--${name}`);
    return index === -1 ? fallback : (args[index + 1] ?? fallback);
};
const has = (name: string) => args.includes(`--${name}`);

const ALCH_TARGETS = flag('alch', '').split(',').map((s) => s.trim()).filter(Boolean);
const MAX_CASTS = Number(flag('max-casts', '0')) || Number.POSITIVE_INFINITY;

await runScript(async ({ bot, sdk }) => {
    await sdk.waitForReady();

    if (ALCH_TARGETS.length === 0) {
        throw new Error('pass --alch "<item>,<item>" — this loop will not guess what to destroy');
    }

    const magic = sdk.getSkill('magic');
    if (!magic || magic.level < SPELLS.HIGH_ALCHEMY.level) {
        throw new Error(
            `magic ${magic?.level ?? '?'} — high alchemy needs 55. Run train-magic.ts first.`,
        );
    }

    // Every bank has an Exchange Clerk beside it (server/content/place-clerks.ts),
    // so "go to a bank" and "go to the bridge" are the same trip.
    if ((has('claim') || has('withdraw-gp')) && !args.includes('--no-travel')) {
        const nearest = NAV.nearestTagged(here(sdk), 'clerk', capabilities(sdk));
        if (nearest) {
            const result = await travelTo(bot, sdk, nearest.place.id);
            console.log(`travel to ${nearest.place.name}: ${result.message}`);
        }
    }

    if (has('claim')) {
        const claimed = await claimDeposits(bot, sdk);
        console.log(`claimed ${claimed.claimed} deposits: ${claimed.message}`);
    }

    const missing = missingRunes(sdk, SPELLS.HIGH_ALCHEMY);
    if (missing.length > 0) {
        // Fire runes are free with a staff, so this is almost always nature runes.
        throw new Error(
            `missing ${missing.join(', ')}. Buy them on chain (nature runes are tokenised, ` +
                `floor about 7 GP) and bridge them in, or equip a staff of fire for the fire runes.`,
        );
    }

    const gpBefore = sdk.countInventoryItems(/^coins$/i);
    const result = await alchInventory(sdk, bot, {
        spell: SPELLS.HIGH_ALCHEMY,
        targets: ALCH_TARGETS,
        maxCasts: MAX_CASTS,
        onCast: ({ casts, item }) => {
            if (casts % 10 === 0) console.log(`  ${casts} casts (last: ${item})`);
        },
    });
    const gpAfter = sdk.countInventoryItems(/^coins$/i);
    const earned = gpAfter - gpBefore;
    console.log(
        `${result.casts} casts, +${earned} gp, +${Math.floor(result.xpGained)} magic xp (${result.reason}: ${result.message})`,
    );

    if (has('withdraw-gp') && earned > 0) {
        const sent = await withdrawGpToWallet(bot, sdk, gpAfter);
        console.log(`withdraw: ${sent.message}`);
    }

    return { casts: result.casts, gpEarned: earned, xpGained: result.xpGained, reason: result.reason };
});

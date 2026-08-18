/**
 * Train Magic to the level High Level Alchemy needs.
 *
 * The route, and why:
 *
 * * **1 to 21** — combat spells on a low-level NPC. Slow (5.5 to 11.5 XP a cast)
 *   and it costs runes, but it is only 5,018 XP. Splashing counts: the SDK treats
 *   Magic XP as the evidence a cast landed, so a miss still trains.
 * * **21 to 55** — Low Level Alchemy, and this is the part worth understanding.
 *   It is the fastest cast in the game at 3 ticks, gives 31 XP, and pays
 *   `0.4 x cost` for the item it destroys. An item bought at its on-chain floor
 *   cost `0.36 x cost`. So from level 21 the training **pays for itself**, and on
 *   expensive items it is outright profitable — 161,142 XP is about 5,200 casts,
 *   which is roughly two and a half hours.
 *
 * Feed it items with `mercbot`: buy a stack near the floor, bridge them in
 * through the Exchange Clerk, and let this burn through them.
 *
 * Usage (from a mercantile checkout, with this directory linked into `bots/`):
 *   bun bots/<name>/train-magic.ts --target 55 --alch "rune platebody"
 */

import { runScript } from '../../sdk/runner';
import { alchInventory, missingRunes } from './lib/alch';
import { SPELLS, bestTrainingSpell, castSeconds, castsToLevel, xpForLevel } from './lib/spells';

const args = process.argv.slice(2);
const flag = (name: string, fallback: string) => {
    const index = args.indexOf(`--${name}`);
    return index === -1 ? fallback : (args[index + 1] ?? fallback);
};

const TARGET_LEVEL = Number(flag('target', '55'));
/** Items to low-alch, most preferred first. */
const ALCH_TARGETS = flag('alch', '').split(',').map((s) => s.trim()).filter(Boolean);
/** NPC to cast combat spells at while below level 21. */
const SPLASH_TARGET = flag('npc', 'chicken');

await runScript(async ({ bot, sdk }) => {
    await bot.skipTutorial();
    await sdk.waitForReady();

    for (;;) {
        const magic = sdk.getSkill('magic');
        if (!magic) throw new Error('no magic skill in the world state — is the bot in game?');
        if (magic.level >= TARGET_LEVEL) {
            console.log(`magic ${magic.level} — target ${TARGET_LEVEL} reached`);
            return { level: magic.level, experience: magic.experience };
        }

        const alchable = ALCH_TARGETS.some((pattern) => sdk.findInventoryItem(pattern));
        const spell = bestTrainingSpell(magic.level, alchable);
        const remaining = castsToLevel(spell, magic.experience, TARGET_LEVEL);
        console.log(
            `magic ${magic.level} (${Math.floor(magic.experience)} xp) — ` +
                `${remaining} casts of ${spellName(spell.component)} to ${TARGET_LEVEL}, ` +
                `about ${((remaining * castSeconds(spell)) / 3600).toFixed(1)}h`,
        );

        const missing = missingRunes(sdk, spell);
        if (missing.length > 0) {
            console.log(`out of ${missing.join(', ')} — restock and run again`);
            return { level: magic.level, blocked: missing };
        }

        if (spell === SPELLS.LOW_ALCHEMY) {
            const result = await alchInventory(sdk, bot, {
                spell: SPELLS.LOW_ALCHEMY,
                targets: ALCH_TARGETS,
                onCast: ({ casts, xpGained }) => {
                    if (casts % 25 === 0) console.log(`  ${casts} casts, +${Math.floor(xpGained)} xp`);
                },
            });
            console.log(`  ${result.reason}: ${result.message}`);
            if (result.reason !== 'out_of_targets' || result.casts === 0) {
                // Out of items is the one case worth looping on — the caller can
                // top the inventory up. Anything else needs a human.
                return { level: sdk.getSkill('magic')?.level, ...result };
            }
            return { level: sdk.getSkill('magic')?.level, ...result };
        }

        // Below level 21: cast at something harmless until the next threshold.
        const nextLevel = Math.min(TARGET_LEVEL, nextThreshold(magic.level));
        console.log(`  casting at ${SPLASH_TARGET} until magic ${nextLevel}`);
        const stopAt = xpForLevel(nextLevel);
        let casts = 0;
        while ((sdk.getSkill('magic')?.experience ?? 0) < stopAt) {
            const result = await bot.castSpell(SPLASH_TARGET, spell.component);
            if (!result.success) {
                console.log(`  cast failed (${result.reason ?? 'unknown'}): ${result.message}`);
                if (result.reason === 'no_runes') return { level: sdk.getSkill('magic')?.level, blocked: 'runes' };
                // A missing or dead target is worth retrying; the NPC respawns.
                await new Promise((resolve) => setTimeout(resolve, 2_000));
                continue;
            }
            casts += 1;
            if (casts % 25 === 0) {
                console.log(`  ${casts} casts, magic ${sdk.getSkill('magic')?.level}`);
            }
        }
    }
});

/** The next level at which a better training option unlocks. */
function nextThreshold(level: number): number {
    for (const threshold of [5, 9, 13, 19, 21, 55]) {
        if (level < threshold) return threshold;
    }
    return 55;
}

function spellName(component: number): string {
    const found = Object.entries(SPELLS).find(([, spell]) => spell.component === component);
    return found ? found[0].toLowerCase().replace(/_/g, ' ') : `component ${component}`;
}

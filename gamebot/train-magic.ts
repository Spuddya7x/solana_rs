/**
 * Train Magic to the level High Level Alchemy needs — and then past it.
 *
 * The route, and why each leg is what it is:
 *
 * * **1 to 3** — Wind Strike, 5.5 XP a cast. 174 XP, about 32 casts. The only
 *   genuinely wasted minute in the whole plan.
 * * **3 to 21** — **splash** Confuse, then Weaken, then Curse: 13, 21 and 29 XP
 *   a cast against Wind Strike's 5.5. These spells refuse to cast on an NPC
 *   whose stats are already lowered, so they are only repeatable while every
 *   cast *misses* — which a bronze kit guarantees. See `lib/splash.ts`. About
 *   250 casts, a quarter of an hour, and roughly 6,000 GP of runes from Aubury.
 * * **21 to 55** — Low Level Alchemy on items bought at their on-chain floor.
 *   31 XP at 3 ticks, and it pays `0.4 x cost` for items that cost
 *   `0.36 x cost`. This leg funds itself.
 * * **55 to 66** — keep going, now with High Level Alchemy, which pays
 *   `0.6 x cost` at 65 XP a cast. 66 is not an arbitrary target: it is the
 *   Wizards' Guild door, and the guild sells **1,000 restocking nature runes**.
 *   Until then the only rune supply is a hundred-unit on-chain pool, so 66 is
 *   what turns this from a trickle into an operation.
 *
 * Usage (from a mercantile checkout, with this directory copied into `bots/`):
 *   bun bots/<name>/train-magic.ts --target 21 --npc chicken
 *   bun bots/<name>/train-magic.ts --target 55 --alch "rune platebody"
 */

import { runScript } from '../../sdk/runner';
import { alchInventory, missingRunes } from './lib/alch';
import {
    SPELLS,
    bestTrainingSpell,
    castSeconds,
    castsToLevel,
    shouldSplash,
    xpForLevel,
} from './lib/spells';
import { SPLASH_KIT, equippedMagicAttack, splashGuaranteed, unwornKit } from './lib/splash';
import { NAV, capabilities, here, travelTo } from './lib/nav';

const args = process.argv.slice(2);
const flag = (name: string, fallback: string) => {
    const index = args.indexOf(`--${name}`);
    return index === -1 ? fallback : (args[index + 1] ?? fallback);
};

const TARGET_LEVEL = Number(flag('target', '55'));
/** Items to low-alch once level 21 is in hand, most preferred first. */
const ALCH_TARGETS = flag('alch', '').split(',').map((s) => s.trim()).filter(Boolean);
/** What to splash. Anything harmless and reliably present will do. */
const SPLASH_TARGET = flag('npc', 'chicken');
/**
 * An undead target to splash Crumble Undead on from level 39 — a skeleton or
 * zombie in the Varrock sewers, say. Without one the ladder tops out at Curse.
 */
const UNDEAD_TARGET = flag('undead', '');
/** Travel to a suitable spot before training, using the navigation graph. */
const TRAVEL = !args.includes('--no-travel');
/** Refuse to cast a stat-reduction spell without the gear to guarantee a miss. */
const REQUIRE_SPLASH_GEAR = !args.includes('--allow-hits');

await runScript(async ({ bot, sdk }) => {
    await bot.skipTutorial();
    await sdk.waitForReady();
    await equipSplashKit();
    await travelToTrainingGround();

    for (;;) {
        const magic = sdk.getSkill('magic');
        if (!magic) throw new Error('no magic skill in the world state — is the bot in game?');
        if (magic.level >= TARGET_LEVEL) {
            console.log(`magic ${magic.level} — target ${TARGET_LEVEL} reached`);
            return { level: magic.level, experience: magic.experience };
        }

        const alchable = ALCH_TARGETS.some((pattern) => sdk.findInventoryItem(pattern));
        const spell = bestTrainingSpell(magic.level, alchable, UNDEAD_TARGET !== '');
        const remaining = castsToLevel(spell, magic.experience, TARGET_LEVEL);
        console.log(
            `magic ${magic.level} (${Math.floor(magic.experience)} xp) — ` +
                `${remaining} casts of ${spellName(spell)} to ${TARGET_LEVEL}, ` +
                `about ${((remaining * castSeconds(spell)) / 3600).toFixed(1)}h`,
        );

        const missing = missingRunes(sdk, spell);
        if (missing.length > 0) {
            console.log(
                `out of ${missing.join(', ')} — Aubury (Varrock) and Betty (Port Sarim) both ` +
                    `stock air, water, earth, mind and body runes with no requirements`,
            );
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
            // Out of items is the normal end of a pass: buy more with mercbot,
            // bridge them in, and run this again.
            return { level: sdk.getSkill('magic')?.level, ...result };
        }

        const splashing = shouldSplash(spell);
        if (splashing && REQUIRE_SPLASH_GEAR && !splashGuaranteed(sdk)) {
            const bonus = equippedMagicAttack(sdk);
            console.log(
                `magic attack bonus is ${bonus}; ${spellName(spell)} needs -64 or worse to be ` +
                    `sure of missing. Wear ${SPLASH_KIT.join(', ')} (about 400 gp of shop value, ` +
                    `-69 together), or pass --allow-hits to train anyway.`,
            );
            return { level: magic.level, blocked: 'splash gear' };
        }

        const nextLevel = Math.min(TARGET_LEVEL, nextThreshold(magic.level));
        // Crumble Undead only affects skeletons, zombies, ghosts and shades.
        const target = spell === SPELLS.CRUMBLE_UNDEAD ? UNDEAD_TARGET : SPLASH_TARGET;
        console.log(
            `  ${splashing ? 'splashing' : 'casting'} ${spellName(spell)} at ${target} until magic ${nextLevel}`,
        );
        const stopAt = xpForLevel(nextLevel);
        let casts = 0;
        let hits = 0;
        while ((sdk.getSkill('magic')?.experience ?? 0) < stopAt) {
            const result = await bot.castSpell(target, spell.component);
            if (!result.success) {
                console.log(`  cast failed (${result.reason ?? 'unknown'}): ${result.message}`);
                if (result.reason === 'no_runes') {
                    return { level: sdk.getSkill('magic')?.level, blocked: 'runes' };
                }
                // A dead or missing target is worth waiting out; chickens respawn.
                await new Promise((resolve) => setTimeout(resolve, 2_000));
                continue;
            }
            casts += 1;
            if (result.hit) hits += 1;
            // A landed stat-reduction spell debuffs the NPC, and every following
            // cast is refused until it restores — so a hit is a warning, not a bonus.
            if (splashing && result.hit && hits === 1) {
                console.log(
                    `  landed a hit — the target is now debuffed and will refuse the next cast. ` +
                        `Check the splash kit (bonus ${equippedMagicAttack(sdk)}).`,
                );
            }
            if (casts % 25 === 0) {
                console.log(`  ${casts} casts (${hits} landed), magic ${sdk.getSkill('magic')?.level}`);
            }
        }
    }

    /**
     * Walk to somewhere with the right targets.
     *
     * Crumble Undead needs skeletons or zombies; everything else just needs
     * something harmless. The navigation graph knows where both are, so the
     * script does not carry coordinates.
     */
    async function travelToTrainingGround(): Promise<void> {
        if (!TRAVEL) return;
        const level = sdk.getSkill('magic')?.level ?? 1;
        const tag = UNDEAD_TARGET !== '' && level >= SPELLS.CRUMBLE_UNDEAD.level ? 'undead' : 'chickens';
        const nearest = NAV.nearestTagged(here(sdk), tag, capabilities(sdk));
        if (!nearest) {
            console.log(`no ${tag} site in the gazetteer — training wherever the bot is standing`);
            return;
        }
        const result = await travelTo(bot, sdk, nearest.place.id);
        console.log(`travel to ${nearest.place.name}: ${result.message}`);
    }

    /** Wear whatever splash gear is in the inventory. */
    async function equipSplashKit(): Promise<void> {
        for (const piece of unwornKit(sdk)) {
            const worn = await bot.equipItem(new RegExp(`^${piece}$`, 'i'));
            console.log(`  equip ${piece}: ${worn.success ? 'ok' : worn.message}`);
        }
        const bonus = equippedMagicAttack(sdk);
        if (bonus < 0) {
            console.log(`magic attack bonus ${bonus} (${bonus <= -64 ? 'splash guaranteed' : 'hits still possible'})`);
        }
    }
});

/** The next level at which a better training option unlocks. */
function nextThreshold(level: number): number {
    for (const threshold of [3, 11, 19, 21, 39, 55, 66]) {
        if (level < threshold) return threshold;
    }
    return 99;
}

function spellName(spell: { component: number }): string {
    const found = Object.entries(SPELLS).find(([, s]) => s.component === spell.component);
    return found ? found[0].toLowerCase().replace(/_/g, ' ') : `component ${spell.component}`;
}

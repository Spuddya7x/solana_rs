/**
 * The whole account, end to end.
 *
 * Fire this at a fresh character and it works out what stage it is at, does the
 * next thing, and repeats:
 *
 * ```
 *   equip     get a splash kit, so stat-reduction spells stay castable
 *   stock     buy runes from a shop that has no entry requirements
 *   train     splash up the ladder to the level alchemy needs
 *   produce   high alch bridged-in items into GP
 *   cash out  send the GP to the wallet, where mercbot turns it into rares
 * ```
 *
 * The stage is **derived from what it observes, not remembered**. There is no
 * saved state to go stale, so the script is safe to kill and restart, safe to
 * run on an account someone has been playing by hand, and safe to run on a
 * hundred accounts at different stages with the same command line.
 *
 * Usage:
 *   bun bots/<name>/campaign.ts --target 55 --alch "rune platebody"
 *   bun bots/<name>/campaign.ts --target 66 --undead skeleton --cycles 20
 */

import { runScript } from '../../sdk/runner';
import { alchInventory, missingRunes } from './lib/alch';
import { NAV, capabilities, here, travelTo } from './lib/nav';
import { SPELLS, bestTrainingSpell, castsToLevel, shouldSplash } from './lib/spells';
import { SPLASH_KIT, equippedMagicAttack, splashGuaranteed, unwornKit } from './lib/splash';
import { runeBudget, stockRunes } from './lib/supply';
import { claimDeposits, withdrawGpToWallet } from './lib/bridge';

const args = process.argv.slice(2);
const flag = (name: string, fallback: string) => {
    const i = args.indexOf(`--${name}`);
    return i === -1 ? fallback : (args[i + 1] ?? fallback);
};

const TARGET_LEVEL = Number(flag('target', '55'));
const ALCH_TARGETS = flag('alch', '').split(',').map((s) => s.trim()).filter(Boolean);
const SPLASH_TARGET = flag('npc', 'chicken');
const UNDEAD_TARGET = flag('undead', 'skeleton');
/** Stop after this many stage transitions, so an unattended run is bounded. */
const MAX_CYCLES = Number(flag('cycles', '50'));
/** Send GP to the wallet once the character is carrying this much. */
const CASH_OUT_AT = Number(flag('cash-out-at', '50000'));
/** Casts to buy runes for in one shopping trip. */
const RESTOCK_CASTS = Number(flag('restock', '250'));

type Stage = 'equip' | 'stock' | 'train' | 'produce' | 'cash_out' | 'done';

await runScript(async ({ bot, sdk }) => {
    await bot.skipTutorial();
    await sdk.waitForReady();

    const log: string[] = [];
    for (let cycle = 0; cycle < MAX_CYCLES; cycle++) {
        const stage = decide();
        const magic = sdk.getSkill('magic');
        console.log(
            `\n[${cycle + 1}/${MAX_CYCLES}] ${stage} — magic ${magic?.level ?? '?'} ` +
                `(${Math.floor(magic?.experience ?? 0)} xp), ${sdk.countInventoryItems(/^coins$/i)} gp`,
        );
        if (stage === 'done') {
            log.push('done');
            break;
        }
        const outcome = await run(stage);
        log.push(`${stage}: ${outcome}`);
        console.log(`  ${outcome}`);
    }

    return {
        level: sdk.getSkill('magic')?.level,
        gp: sdk.countInventoryItems(/^coins$/i),
        log,
    };

    /**
     * Work out what this account needs next.
     *
     * Ordered by what blocks what: no kit blocks splashing, no runes blocks
     * casting, and both block training. Producing outranks cashing out because
     * a full inventory of alchables is worth more converted than carried.
     */
    function decide(): Stage {
        const level = sdk.getSkill('magic')?.level ?? 1;
        const spell = bestTrainingSpell(level, hasAlchables(), UNDEAD_TARGET !== '');
        const trained = level >= TARGET_LEVEL;

        if (!trained && shouldSplash(spell) && !splashGuaranteed(sdk) && canImproveKit()) {
            return 'equip';
        }
        if (!trained && missingRunes(sdk, spell).some((rune) => rune !== 'naturerune')) {
            return 'stock';
        }
        if (!trained) return 'train';
        if (level >= SPELLS.HIGH_ALCHEMY.level && hasAlchables()) return 'produce';
        if (sdk.countInventoryItems(/^coins$/i) >= CASH_OUT_AT) return 'cash_out';
        return 'done';
    }

    async function run(stage: Stage): Promise<string> {
        switch (stage) {
            case 'equip':
                return equip();
            case 'stock':
                return stock();
            case 'train':
                return train();
            case 'produce':
                return produce();
            case 'cash_out':
                return cashOut();
            case 'done':
                return 'nothing to do';
        }
    }

    /** Wear the splash kit, buying the missing pieces if there is GP for them. */
    async function equip(): Promise<string> {
        for (const piece of unwornKit(sdk)) {
            await bot.equipItem(new RegExp(`^${piece}$`, 'i'));
        }
        if (splashGuaranteed(sdk)) return `magic attack ${equippedMagicAttack(sdk)} — splash guaranteed`;

        // The whole kit is about 400 GP of shop value, and the penalty does not
        // scale with tier, so bronze is as good as rune here.
        const missing = SPLASH_KIT.filter((piece) => !sdk.findInventoryItem(new RegExp(`^${piece}$`, 'i')));
        const smith = NAV.nearestTagged(here(sdk), 'shop', capabilities(sdk));
        if (!smith || sdk.countInventoryItems(/^coins$/i) < 1_000) {
            return `need ${missing.join(', ')} and ${1_000} gp to buy them (have ${sdk.countInventoryItems(/^coins$/i)})`;
        }
        await travelTo(bot, sdk, smith.place.id);
        if ((await bot.openShop()).success) {
            for (const piece of missing) {
                await bot.buyFromShop(new RegExp(`^${piece}$`, 'i'), 1);
            }
            await bot.closeShop();
        }
        for (const piece of unwornKit(sdk)) {
            await bot.equipItem(new RegExp(`^${piece}$`, 'i'));
        }
        return `magic attack ${equippedMagicAttack(sdk)}`;
    }

    /** Buy runes for the spell this account is about to train with. */
    async function stock(): Promise<string> {
        const level = sdk.getSkill('magic')?.level ?? 1;
        const spell = bestTrainingSpell(level, hasAlchables(), UNDEAD_TARGET !== '');
        const wanted = Math.min(RESTOCK_CASTS, castsToLevel(spell, sdk.getSkill('magic')?.experience ?? 0, TARGET_LEVEL));
        const budget = runeBudget(spell, wanted);
        const coins = sdk.countInventoryItems(/^coins$/i);
        if (coins < budget) {
            // Runes are the only running cost of training, and the chain side is
            // where the GP lives, so this is the moment to bridge some in.
            const claimed = await claimDeposits(bot, sdk);
            if (claimed.claimed === 0 && sdk.countInventoryItems(/^coins$/i) < budget / 4) {
                return `need about ${budget} gp of runes and have ${coins} — deposit GP on chain and claim it`;
            }
        }
        const result = await stockRunes(bot, sdk, spell, wanted);
        return result.message;
    }

    /** Splash or cast up the ladder until the next stage unlocks. */
    async function train(): Promise<string> {
        const level = sdk.getSkill('magic')?.level ?? 1;
        const spell = bestTrainingSpell(level, hasAlchables(), UNDEAD_TARGET !== '');
        if (spell === SPELLS.LOW_ALCHEMY) {
            const result = await alchInventory(sdk, bot, { spell, targets: ALCH_TARGETS });
            return `${result.casts} low alchs, +${Math.floor(result.xpGained)} xp (${result.reason})`;
        }

        const undead = spell === SPELLS.CRUMBLE_UNDEAD;
        const target = undead ? UNDEAD_TARGET : SPLASH_TARGET;
        const site = NAV.nearestTagged(here(sdk), undead ? 'undead' : 'chickens', capabilities(sdk));
        if (site) await travelTo(bot, sdk, site.place.id);

        let casts = 0;
        let landed = 0;
        // Cast until the runes run out or the level target is met; the outer loop
        // re-decides the stage afterwards.
        while (missingRunes(sdk, spell).length === 0 && (sdk.getSkill('magic')?.level ?? 1) < TARGET_LEVEL) {
            const result = await bot.castSpell(target, spell.component);
            if (!result.success) {
                if (result.reason === 'no_runes') break;
                await new Promise((resolve) => setTimeout(resolve, 2_000));
                continue;
            }
            casts++;
            if (result.hit) landed++;
            if (casts % 50 === 0) console.log(`    ${casts} casts, magic ${sdk.getSkill('magic')?.level}`);
        }
        const warning =
            shouldSplash(spell) && landed > 0
                ? ` — ${landed} landed, which blocks the next cast; check the kit (${equippedMagicAttack(sdk)})`
                : '';
        return `${casts} casts of ${spell === SPELLS.CRUMBLE_UNDEAD ? 'crumble undead' : 'a training spell'}${warning}`;
    }

    /** Turn bridged-in items into GP. */
    async function produce(): Promise<string> {
        const claimed = await claimDeposits(bot, sdk);
        if (claimed.claimed > 0) console.log(`    claimed ${claimed.claimed} deposits`);
        const missing = missingRunes(sdk, SPELLS.HIGH_ALCHEMY);
        if (missing.length > 0) {
            return `missing ${missing.join(', ')} — nature runes come from the chain until 66 Magic`;
        }
        const before = sdk.countInventoryItems(/^coins$/i);
        const result = await alchInventory(sdk, bot, { spell: SPELLS.HIGH_ALCHEMY, targets: ALCH_TARGETS });
        return `${result.casts} high alchs, +${sdk.countInventoryItems(/^coins$/i) - before} gp (${result.reason})`;
    }

    /** Send the GP to the wallet, where the on-chain side buys scarcity with it. */
    async function cashOut(): Promise<string> {
        const coins = sdk.countInventoryItems(/^coins$/i);
        const result = await withdrawGpToWallet(bot, sdk, coins);
        return result.message;
    }

    function hasAlchables(): boolean {
        return ALCH_TARGETS.some((pattern) => sdk.findInventoryItem(pattern));
    }

    function canImproveKit(): boolean {
        return unwornKit(sdk).length > 0 || sdk.countInventoryItems(/^coins$/i) >= 1_000;
    }
});

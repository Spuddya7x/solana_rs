/**
 * Splashing: casting a spell that is guaranteed to miss, on purpose.
 *
 * This is the fastest way to train Magic from 1 to 21 in this world, and it
 * works because of three things in the engine, each verified in the server
 * scripts rather than assumed:
 *
 * 1. **XP is paid before the hit roll.** `pvm_default_spell` calls
 *    `~pvm_spell_cast` — which deletes the runes and calls `~give_spell_xp` —
 *    *then* rolls `~player_npc_hit_roll`. A splash keeps the full base XP.
 *    (`server/content/scripts/skill_combat/scripts/player/player_magic.rs2`)
 *
 * 2. **A splash never applies the debuff.** `~pvm_stat_change_effect` only runs
 *    on the success branch. That matters because `~pvm_debuff_allowed` refuses
 *    to cast Confuse/Weaken/Curse on an NPC whose stat is *already* lowered —
 *    "Your foe's attack has already been weakened." So landing a hit is what
 *    stops you training, and missing forever is what lets you keep going. One
 *    chicken will do for the whole grind.
 *
 * 3. **A magic attack bonus of -64 or worse guarantees the miss.** The roll is
 *    `combat_stat(effective_magic, bonus) = effective_magic * (bonus + 64)`, and
 *    a hit needs `randominc(attack) > randominc(defence)`. At `bonus <= -64` the
 *    attack roll is zero or negative, `randominc` of which is never above a
 *    defence roll of zero or more. (`combat.rs2`, `player_combat.rs2`)
 *
 * The stat-reduction spells are worth far more XP than the strike spells at the
 * same level — Curse is 29 XP against Wind Strike's 5.5 — and they are exactly
 * the ones that only stay castable if you keep missing.
 */

import type { BotSDK } from '../../../sdk/index';
import type { InventoryItem } from '../../../sdk/types';

/** Magic attack bonus at or below which every cast splashes. */
export const GUARANTEED_SPLASH_BONUS = -64;

/**
 * Magic attack penalties by item, read from the game's `param=magicattack`.
 *
 * Only the pieces worth wearing for this are listed: metal armour carries the
 * heaviest penalties, and bronze carries the same penalty as rune for a
 * four-hundredth of the price.
 */
export const MAGIC_ATTACK_PENALTIES: Readonly<Record<string, number>> = {
    // torso -30, legs -21, shield -8, helm -6, weapon -4 = -69
    'bronze platebody': -30,
    'iron platebody': -30,
    'steel platebody': -30,
    'bronze platelegs': -21,
    'iron platelegs': -21,
    'steel platelegs': -21,
    'bronze kiteshield': -8,
    'iron kiteshield': -8,
    'steel kiteshield': -8,
    'bronze full helm': -6,
    'iron full helm': -6,
    'steel full helm': -6,
    'bronze warhammer': -4,
    'iron warhammer': -4,
};

/**
 * The cheapest kit that clears -64, and where to buy it.
 *
 * Bronze, because the magic penalty does not scale with tier: a bronze
 * platebody and a rune platebody are both -30, and the bronze one costs 160 GP
 * of shop value against 65,000.
 */
export const SPLASH_KIT = [
    'bronze platebody',
    'bronze platelegs',
    'bronze kiteshield',
    'bronze full helm',
    'bronze warhammer',
] as const;

/** Sum of the magic attack penalties the bot is currently wearing. */
export function equippedMagicAttack(sdk: BotSDK): number {
    const worn: InventoryItem[] = sdk.getState()?.equipment ?? [];
    return worn.reduce((total, item) => {
        if (!item?.name) return total;
        return total + (MAGIC_ATTACK_PENALTIES[item.name.toLowerCase()] ?? 0);
    }, 0);
}

/** Whether the current kit guarantees a splash. */
export function splashGuaranteed(sdk: BotSDK): boolean {
    return equippedMagicAttack(sdk) <= GUARANTEED_SPLASH_BONUS;
}

/** Kit pieces that are in the inventory but not yet worn. */
export function unwornKit(sdk: BotSDK): string[] {
    const worn = new Set(
        (sdk.getState()?.equipment ?? []).map((item) => item?.name?.toLowerCase()).filter(Boolean),
    );
    return SPLASH_KIT.filter(
        (piece) => !worn.has(piece) && sdk.findInventoryItem(new RegExp(`^${piece}$`, 'i')),
    );
}

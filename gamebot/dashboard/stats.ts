/**
 * Turning a stream of world states into a session report.
 *
 * Everything the dashboard shows is *derived* — the bot scripts do not report
 * anything and do not know the dashboard exists. That is deliberate: the gateway
 * lets an SDK client connect in `observe` mode, which is read-only, never
 * pre-empts the controller and is never pre-empted by it, and multiple observers
 * coexist freely. So a watcher can attach to any running bot, including one
 * started before the dashboard was, and detach without disturbing it.
 *
 * The cost of that independence is that every number here is a difference
 * between two observed frames rather than something the script counted. Two
 * consequences worth knowing:
 *
 * * **Coins are inventory, not a ledger.** Selling to a shop, dropping GP or
 *   dying all move the number, and none of them are "earnings". The session
 *   tracks the running total honestly and labels it *carried*, not *profit*.
 * * **XP is monotonic, so it is the trustworthy axis.** Experience only ever
 *   goes up, which makes it the one rate that cannot be confused by an
 *   inventory shuffle. When the two disagree, believe the XP.
 */

import { xpForLevel } from '../lib/spells';

/** The subset of `BotWorldState` this module needs — kept structural so tests
 * can build frames without importing the SDK. */
export interface StateFrame {
    tick: number;
    inGame: boolean;
    player: {
        name: string;
        combatLevel: number;
        hp: number;
        maxHp: number;
        /**
         * World tiles — the space every coordinate in this bot is expressed in,
         * and the space NPCs and locs already report their own `x`/`z` in.
         *
         * `PlayerState` also carries plain `x`/`z`, and those are **not** the
         * same thing: standing in Lumbridge they read 6976,6976 against a
         * worldX/worldZ of 3222,3222. Mixing the two silently produces
         * distances that are wrong by thousands of tiles, which is exactly what
         * this field being named unambiguously is here to prevent.
         */
        worldX: number;
        worldZ: number;
        level: number;
        runEnergy: number;
    } | null;
    skills: { name: string; level: number; baseLevel: number; experience: number }[];
    inventory: { slot: number; id: number; name: string; count: number }[];
    gameMessages?: { type: number; text: string; sender: string; tick: number }[];
}

/** One skill's progress over the session. */
export interface SkillProgress {
    name: string;
    /**
     * The **true** level, from `baseLevel`.
     *
     * `SkillState.level` is the current value after boosts and drains, which is
     * not the level the XP curve is measured against. It matters more here than
     * anywhere: a thieving bot's Hitpoints are drained essentially all the time,
     * so reading `level` would show it as a lower-levelled account and compute a
     * next-level target it passed hours ago.
     */
    level: number;
    /** The boosted or drained value, when it differs from [`level`]. */
    current: number;
    /** Level when the dashboard attached. */
    startLevel: number;
    experience: number;
    /** XP gained since attaching. */
    gained: number;
    /** Projected XP per hour at the session's average rate. */
    perHour: number;
    /** XP still needed for the next level, or null at 99. */
    toNextLevel: number | null;
    /** Seconds to the next level at the current rate, or null if not advancing. */
    secondsToLevel: number | null;
}

/** What the dashboard renders. */
export interface Session {
    /** Bot name, once a frame has arrived. */
    name: string | null;
    combatLevel: number;
    /** Wall-clock seconds since the dashboard attached. */
    runtimeSeconds: number;
    /** Frames received — a stalled feed shows as this not moving. */
    frames: number;
    tick: number;
    inGame: boolean;
    position: { x: number; z: number; level: number } | null;
    hp: { current: number; max: number } | null;
    runEnergy: number;
    /** Coins carried now, and the change since attaching. */
    coins: number;
    coinsGained: number;
    coinsPerHour: number;
    /** Every skill that has moved, best rate first, then the rest. */
    skills: SkillProgress[];
    /** Total XP across all skills, and its rate. */
    totalXpGained: number;
    totalXpPerHour: number;
    /** Inventory as 28 slots, nulls included, so the grid renders faithfully. */
    inventory: ({ name: string; count: number; id: number } | null)[];
    /** What the bot appears to be doing, inferred from recent game messages. */
    activity: string;
    /** Recent notable events, newest last. */
    log: { at: number; text: string; kind: LogKind }[];
}

export type LogKind = 'levelup' | 'loot' | 'damage' | 'action' | 'system';

/** Inventory slots the game gives a player. */
export const INVENTORY_SLOTS = 28;

/** How many log lines to keep. */
const LOG_LIMIT = 200;

/**
 * Message patterns worth surfacing, and what they mean.
 *
 * Ordered: the first match wins, so put the specific before the general. These
 * are the engine's own `mes(...)` strings — `pick_pocket` says "You pick the
 * man's pocket", `~fail_pick_pocket` says "You fail to pick the pocket".
 */
const ACTIVITY_PATTERNS: { re: RegExp; activity: string; kind: LogKind }[] = [
    { re: /you pick the .*pocket/i, activity: 'Thieving', kind: 'loot' },
    { re: /you fail to pick|you have been stunned/i, activity: 'Thieving (stunned)', kind: 'damage' },
    { re: /you catch some shrimps?|you catch a/i, activity: 'Fishing', kind: 'loot' },
    { re: /you cast out your (net|line)/i, activity: 'Fishing', kind: 'action' },
    { re: /the fire catches/i, activity: 'Firemaking', kind: 'action' },
    { re: /you attempt to light/i, activity: 'Firemaking', kind: 'action' },
    { re: /nicely cooked|you burn/i, activity: 'Cooking', kind: 'action' },
    { re: /you get some logs/i, activity: 'Woodcutting', kind: 'loot' },
    { re: /you swing your axe/i, activity: 'Woodcutting', kind: 'action' },
    { re: /you convert .* into .* coins/i, activity: 'Alching', kind: 'loot' },
    { re: /advanced .* level|congratulations/i, activity: 'Levelled up', kind: 'levelup' },
    { re: /oh dear, you are dead/i, activity: 'DEAD', kind: 'damage' },
];

/** Accumulates frames into a [`Session`]. */
export class SessionTracker {
    private startedAt: number;
    private startXp = new Map<string, number>();
    private startLevel = new Map<string, number>();
    private startCoins: number | null = null;
    private log: { at: number; text: string; kind: LogKind }[] = [];
    private activity = 'Idle';
    private frames = 0;
    private lastMessageTick = -1;
    private lastLevel = new Map<string, number>();
    private last: StateFrame | null = null;

    constructor(private now: () => number = Date.now) {
        this.startedAt = now();
    }

    /** Fold one observed frame in. */
    push(frame: StateFrame): void {
        this.frames++;
        this.last = frame;

        for (const skill of frame.skills) {
            // First sighting fixes the session baseline; it is never a level-up.
            if (!this.startXp.has(skill.name)) {
                this.startXp.set(skill.name, skill.experience);
                this.startLevel.set(skill.name, skill.baseLevel);
                this.lastLevel.set(skill.name, skill.baseLevel);
                continue;
            }
            // Log level-ups off the level itself rather than the game's message:
            // the message feed is a bounded rolling window, so a busy tick can
            // push one out, but a level cannot be missed.
            // Off `baseLevel`, so a hitpoint coming back after a stun is not
            // announced as a level-up every single time the bot eats.
            const previous = this.lastLevel.get(skill.name) ?? skill.baseLevel;
            if (skill.baseLevel > previous) {
                this.append(`${skill.name} level ${skill.baseLevel}`, 'levelup');
            }
            this.lastLevel.set(skill.name, skill.baseLevel);
        }

        const coins = countCoins(frame.inventory);
        if (this.startCoins === null) this.startCoins = coins;

        for (const message of frame.gameMessages ?? []) {
            // The feed is a rolling window, so the same message reappears on
            // later frames; the tick is what makes it new.
            if (message.tick <= this.lastMessageTick) continue;
            this.lastMessageTick = message.tick;
            const matched = ACTIVITY_PATTERNS.find((p) => p.re.test(message.text));
            if (matched) {
                this.activity = matched.activity;
                this.append(message.text, matched.kind);
            }
        }
    }

    /** Note something the bot's own script said, if it chooses to say anything. */
    append(text: string, kind: LogKind = 'system'): void {
        this.log.push({ at: this.now(), text, kind });
        if (this.log.length > LOG_LIMIT) this.log.splice(0, this.log.length - LOG_LIMIT);
    }

    /** Render the current session. */
    snapshot(): Session {
        const runtimeSeconds = Math.max(0, (this.now() - this.startedAt) / 1000);
        const hours = runtimeSeconds / 3600;
        const frame = this.last;

        const skills: SkillProgress[] = (frame?.skills ?? []).map((skill) => {
            const startXp = this.startXp.get(skill.name) ?? skill.experience;
            const gained = Math.max(0, skill.experience - startXp);
            const perHour = hours > 0 ? gained / hours : 0;
            const trueLevel = skill.baseLevel;
            const next = trueLevel >= 99 ? null : xpForLevel(trueLevel + 1) - skill.experience;
            return {
                name: skill.name,
                level: trueLevel,
                current: skill.level,
                startLevel: this.startLevel.get(skill.name) ?? trueLevel,
                experience: skill.experience,
                gained,
                perHour,
                toNextLevel: next === null ? null : Math.max(0, next),
                secondsToLevel: next !== null && perHour > 0 ? (Math.max(0, next) / perHour) * 3600 : null,
            };
        });
        // Skills that are moving first, then the rest alphabetically, so the
        // eye lands on what the run is actually training.
        skills.sort((a, b) => b.perHour - a.perHour || a.name.localeCompare(b.name));

        const coins = frame ? countCoins(frame.inventory) : 0;
        const coinsGained = coins - (this.startCoins ?? coins);
        const totalXpGained = skills.reduce((sum, s) => sum + s.gained, 0);

        return {
            name: frame?.player?.name ?? null,
            combatLevel: frame?.player?.combatLevel ?? 0,
            runtimeSeconds,
            frames: this.frames,
            tick: frame?.tick ?? 0,
            inGame: frame?.inGame ?? false,
            position: frame?.player
                ? { x: frame.player.worldX, z: frame.player.worldZ, level: frame.player.level }
                : null,
            hp: frame?.player ? { current: frame.player.hp, max: frame.player.maxHp } : null,
            runEnergy: frame?.player?.runEnergy ?? 0,
            coins,
            coinsGained,
            coinsPerHour: hours > 0 ? coinsGained / hours : 0,
            skills,
            totalXpGained,
            totalXpPerHour: hours > 0 ? totalXpGained / hours : 0,
            inventory: layOutInventory(frame?.inventory ?? []),
            activity: this.activity,
            log: this.log.slice(-60),
        };
    }
}

/** Coins in an inventory. Stackable, so this is a count not a slot tally. */
export function countCoins(inventory: { name: string; count: number }[]): number {
    return inventory
        .filter((item) => /^coins$/i.test(item.name))
        .reduce((sum, item) => sum + item.count, 0);
}

/**
 * Place items into their real slots.
 *
 * The state feed sends only occupied slots, but the classic interface is a
 * fixed 4x7 grid and a gap in the middle of it is information — it is how you
 * see at a glance that the bot dropped something.
 */
export function layOutInventory(
    items: { slot: number; id: number; name: string; count: number }[],
): ({ name: string; count: number; id: number } | null)[] {
    const grid: ({ name: string; count: number; id: number } | null)[] = Array(INVENTORY_SLOTS).fill(null);
    for (const item of items) {
        if (item.slot >= 0 && item.slot < INVENTORY_SLOTS) {
            grid[item.slot] = { name: item.name, count: item.count, id: item.id };
        }
    }
    return grid;
}

/** `1h 04m 12s`, the way a bot paint has always shown runtime. */
export function formatRuntime(seconds: number): string {
    const s = Math.max(0, Math.floor(seconds));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = s % 60;
    const pad = (n: number) => String(n).padStart(2, '0');
    return h > 0 ? `${h}h ${pad(m)}m ${pad(sec)}s` : `${m}m ${pad(sec)}s`;
}

/** `1.4M`, `12.3K`, `847` — the compact form a paint uses for XP and GP. */
export function formatCount(value: number): string {
    const n = Math.round(value);
    const sign = n < 0 ? '-' : '';
    const abs = Math.abs(n);
    if (abs >= 1_000_000) return `${sign}${(abs / 1_000_000).toFixed(1)}M`;
    if (abs >= 10_000) return `${sign}${Math.round(abs / 1000)}K`;
    if (abs >= 1_000) return `${sign}${(abs / 1000).toFixed(1)}K`;
    return `${sign}${abs}`;
}

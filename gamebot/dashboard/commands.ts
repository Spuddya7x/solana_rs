/**
 * A console for the session window.
 *
 * The dashboard attaches in `observe` mode, and that mode is not a limitation
 * to route around — it is what makes the window safe to open next to a running
 * bot. The gateway is explicit about the alternative:
 *
 * > Controller pre-emption (last controller wins): when a new 'control' mode
 * > client connects, **any existing controllers are disconnected**.
 *
 * So a console that could walk the character would, on its first command, kill
 * whatever script was driving it. That is a real thing to want sometimes, and
 * the wrong thing to do by accident — so commands declare what they need and
 * the dispatcher refuses anything the current connection is not entitled to.
 * Taking control is its own command, and it says what it is about to break.
 *
 * Three tiers:
 *
 * * `none` — pure planning. Runs with no game attached at all: the pickpocket
 *   tables, the shop prices, the forage arithmetic. The same numbers `mercbot`
 *   prints, without leaving the window.
 * * `observe` — reads live world state, plus `say`, which is the single action
 *   the gateway lets an observer send.
 * * `control` — moves the character, and therefore evicts the running script.
 *
 * The registry, the parser and the gate are pure so they can be tested without
 * a gateway; handlers receive a context and are the only part that touches one.
 */

import { SHOPS } from '../lib/money/world.generated';
import { SAFESPOTS } from '../lib/money/safespots';
import { bestClusters, respawnSeconds } from '../lib/money/loot-run';
import { sellPriceFor, willBuy } from '../lib/money/pricing';
import { TARGETS, damagePerHour, healingNeeded, hitpointFloor, successChance } from '../lib/thieving';
import { SHRIMP_HEAL, SHRIMP_SPOT, TREE } from '../lib/forage';
import { FATAL_OUTCOMES as FATAL, classify, randomEventNearby, type Outcome } from '../lib/thieving';
import type { Session } from './stats';

/** What a command needs from the connection to be allowed to run. */
export type Needs = 'none' | 'observe' | 'control';

/** Everything a handler is allowed to reach. */
export interface CommandContext {
    /** The derived session, always present. */
    session: Session;
    /** Live world state, when a frame has arrived. */
    state: Worldish | null;
    /** Await the next frame, so an action can be judged on fresh state. */
    settle?: (ticks: number) => Promise<Worldish | null>;
    /** Current connection mode. */
    mode: Needs;
    /** Send a chat message. Observers may do this; nothing else. */
    say?: (message: string) => Promise<void>;
    /** Escalate to control, evicting the running script. */
    takeControl?: () => Promise<void>;
    /** Drop back to observe. */
    release?: () => Promise<void>;
    /** Actions, present only once the connection holds control. */
    act?: Actions;
}

/**
 * The slice of `BotActions` the console drives.
 *
 * Structural rather than the class itself, so the command tests can exercise
 * every branch — including the failures — without a gateway.
 */
export interface Actions {
    walkTo: (x: number, z: number) => Promise<{ success: boolean; message?: string }>;
    interactNpc: (
        target: string | RegExp,
        option: string | RegExp,
    ) => Promise<{ success: boolean; message?: string }>;
    eatFood: (target: string | RegExp) => Promise<{ success: boolean; message?: string }>;
}

/** The parts of `BotWorldState` the console reads, kept structural for tests. */
export interface Worldish {
    tick: number;
    player: { name: string; x: number; z: number; level: number; hp: number; maxHp: number } | null;
    nearbyNpcs?: { name: string; x: number; z: number }[];
    nearbyLocs?: { name: string; x: number; z: number }[];
    groundItems?: { name: string; x: number; z: number; count?: number }[];
    /** Newest last, as the engine publishes them. */
    gameMessages?: { text: string }[];
    inventory?: { name: string; count: number }[];
}

export interface Command {
    name: string;
    /** `walk <x> <z>` — shown in help. */
    usage: string;
    summary: string;
    needs: Needs;
    run: (args: string[], ctx: CommandContext) => string | Promise<string>;
}

/** Is a connection in `mode` allowed to run something needing `needs`? */
export function permits(mode: Needs, needs: Needs): boolean {
    if (needs === 'none') return true;
    if (needs === 'observe') return mode === 'observe' || mode === 'control';
    return mode === 'control';
}

const pad = (s: string | number, n: number) => String(s).padEnd(n);
const num = (v: number, n = 0) => v.toLocaleString('en-US', { maximumFractionDigits: n });

/** Manhattan tiles, which is how everything else in this bot measures distance. */
const tilesFrom = (a: { x: number; z: number }, b: { x: number; z: number }) =>
    Math.abs(a.x - b.x) + Math.abs(a.z - b.z);

function requireState(ctx: CommandContext): Worldish {
    if (!ctx.state) throw new Error('no world state yet — is the bot logged in?');
    return ctx.state;
}

/** Coins carried, from whichever frame is current. */
function coinsIn(state: Worldish | null): number {
    return state?.inventory?.find((i) => /^coins$/i.test(i.name))?.count ?? 0;
}

function matcher(args: string[]): RegExp | null {
    const pattern = args.join(' ').trim();
    return pattern ? new RegExp(pattern, 'i') : null;
}

export const COMMANDS: Command[] = [
    // ---- planning: no game needed ------------------------------------------
    {
        name: 'thieve',
        usage: 'thieve [level]',
        summary: 'Pickpocket targets and what they cost in hitpoints',
        needs: 'none',
        run(args, ctx) {
            const level = Number(args[0]) || ctx.session.skills.find((s) => s.name === 'Thieving')?.level || 1;
            const lines = [`Thieving ${level} — passive regeneration is 60 hp/hour, and that is the constraint.`, ''];
            lines.push(`${pad('target', 14)}${pad('lvl', 5)}${pad('success', 9)}${pad('hp/hour', 9)}floor`);
            for (const target of TARGETS) {
                const usable = target.level <= level;
                lines.push(
                    `${pad(target.name, 14)}${pad(target.level, 5)}` +
                        `${pad(`${(successChance(target, level) * 100).toFixed(0)}%`, 9)}` +
                        `${pad(num(damagePerHour(target, level)), 9)}` +
                        `${hitpointFloor(target)} hp${usable ? '' : '   (locked)'}`,
                );
            }
            return lines.join('\n');
        },
    },
    {
        name: 'forage',
        usage: 'forage [minutes]',
        summary: 'Food a thieving session needs, and the circuit that makes it',
        needs: 'none',
        run(args, ctx) {
            const minutes = Number(args[0]) || 12;
            const level = ctx.session.skills.find((s) => s.name === 'Thieving')?.level || 1;
            const target = TARGETS.filter((t) => t.level <= level).at(-1) ?? TARGETS[0]!;
            const hp = healingNeeded(target, level, minutes);
            const shrimp = Math.ceil(hp / SHRIMP_HEAL);
            const cooking = ctx.session.skills.find((s) => s.name === 'Cooking')?.level || 1;
            // Cooking 1 keeps about half; the roll is `successchance 128,512`.
            const keep = Math.min(256, Math.floor((128 * (99 - cooking)) / 98) + Math.floor((512 * (cooking - 1)) / 98) + 1) / 256;
            return [
                `${minutes} min on ${target.name} costs ${num(hp)} hp of food after regeneration.`,
                `That is ${shrimp} cooked shrimp; at Cooking ${cooking} you keep ${(keep * 100).toFixed(0)}%,`,
                `so fish about ${Math.ceil(shrimp / Math.max(keep, 0.1))} raw. One inventory is 25 slots = 75 hp.`,
                '',
                `  Lumbridge (3222,3218) --115--> shrimp (${SHRIMP_SPOT.x},${SHRIMP_SPOT.z})`,
                `  tree (${TREE.x},${TREE.z}) sits on the line home, so the fire costs no detour.`,
            ].join('\n');
        },
    },
    {
        name: 'shop',
        usage: 'shop <item> [cost]',
        summary: 'Best reachable counter for an item, and what it pays',
        needs: 'none',
        run(args) {
            const item = args[0];
            if (!item) return 'usage: shop <item> [cost]';
            const cost = Number(args[1]) || 1000;
            const usable = SHOPS.filter((s) => s.accessible && s.safe && willBuy(s, item));
            if (!usable.length) return `nothing reachable buys '${item}'`;
            const ranked = usable
                .map((s) => ({ shop: s, price: sellPriceFor(s, item, cost, 0) }))
                .sort((a, b) => b.price - a.price || (a.shop.walkTiles ?? 1e9) - (b.shop.walkTiles ?? 1e9))
                .slice(0, 6);
            // A general store buys anything, so every item finds *a* buyer at
            // 400/1000 — 40% of cost, which is low-alchemy value and 11% over
            // the 36% pool floor, gone by the third unit. Only a specialist that
            // stocks the item is a trade, so say which one this is.
            const specialist = ranked.find((r) => item in r.shop.stock);
            const verdict = specialist
                ? `specialist: ${specialist.shop.title} at ${specialist.shop.buyMultiplier}/1000`
                : 'no reachable specialist — the rows below are general stores at 40% of cost, which is not a trade';
            return [
                `'${item}' at cost ${num(cost)} — pool floor would be ${num(cost * 0.36)} gp`,
                verdict,
                '',
                ...ranked.map(
                    (r) =>
                        `${pad(num(r.price) + ' gp', 12)}${pad(`${r.shop.buyMultiplier}/1000`, 11)}` +
                        `${pad(`${r.shop.walkTiles ?? '?'}t`, 7)}${r.shop.title}` +
                        `${item in r.shop.stock ? '  *' : ''}`,
                ),
            ].join('\n');
        },
    },
    {
        name: 'spawns',
        usage: 'spawns [count]',
        summary: 'Ground-spawn clusters worth a loot run',
        needs: 'none',
        run(args) {
            const clusters = bestClusters(Number(args[0]) || 4);
            return clusters
                .map(
                    (c) =>
                        `${pad(`(${c.x},${c.z})`, 14)}${pad(num(c.value) + ' gp', 10)}` +
                        `${pad(`${respawnSeconds(c)}s`, 7)}${c.items.map((i) => i.item).join(', ')}`,
                )
                .join('\n');
        },
    },
    {
        name: 'safespots',
        usage: 'safespots',
        summary: 'Verified safespots and what the monster drops',
        needs: 'none',
        run() {
            return SAFESPOTS.map(
                (s) =>
                    `${pad(s.npc, 16)}${pad(num(s.gpPerKill) + ' gp/kill', 14)}` +
                    `stand (${s.stand.x},${s.stand.z}) → ${s.npc} at (${s.target.x},${s.target.z})`,
            ).join('\n');
        },
    },

    // ---- inspection: needs a live feed --------------------------------------
    {
        name: 'skills',
        usage: 'skills [pattern]',
        summary: 'Levels, XP gained and rates this session',
        needs: 'observe',
        run(args, ctx) {
            const re = matcher(args);
            const rows = ctx.session.skills.filter((s) => !re || re.test(s.name));
            if (!rows.length) return 'no skills match';
            return rows
                .map((s) => {
                    const drained = s.current !== s.level ? ` (${s.current})` : '';
                    const rate = s.gained > 0 ? `${num(s.gained)} xp  ${num(s.perHour)}/hr` : '—';
                    return `${pad(s.name, 13)}${pad(s.level + drained, 9)}${rate}`;
                })
                .join('\n');
        },
    },
    {
        name: 'inv',
        usage: 'inv',
        summary: 'Inventory, occupied slots only',
        needs: 'observe',
        run(_args, ctx) {
            const held = ctx.session.inventory
                .map((slot, i) => ({ slot, i }))
                .filter((e) => e.slot !== null);
            if (!held.length) return 'inventory empty';
            return (
                held.map((e) => `${pad(e.i, 4)}${pad(e.slot!.name, 22)}${e.slot!.count > 1 ? num(e.slot!.count) : ''}`).join('\n') +
                `\n${held.length}/28 slots used`
            );
        },
    },
    {
        name: 'where',
        usage: 'where',
        summary: 'Position, hitpoints and what the bot appears to be doing',
        needs: 'observe',
        run(_args, ctx) {
            const s = ctx.session;
            const pos = s.position ? `(${s.position.x}, ${s.position.z}) plane ${s.position.level}` : 'unknown';
            const hp = s.hp ? `${s.hp.current}/${s.hp.max} hp` : '—';
            return [
                `${s.name ?? 'not logged in'} at ${pos}`,
                `${hp} · run ${Math.round(s.runEnergy)}% · tick ${s.tick}`,
                `activity: ${s.activity}`,
                `carrying ${num(s.coins)} gp (${s.coinsGained >= 0 ? '+' : ''}${num(s.coinsGained)} this session)`,
            ].join('\n');
        },
    },
    {
        name: 'npcs',
        usage: 'npcs [pattern]',
        summary: 'Nearby NPCs, nearest first',
        needs: 'observe',
        run(args, ctx) {
            const state = requireState(ctx);
            const re = matcher(args);
            const me = state.player;
            const found = (state.nearbyNpcs ?? [])
                .filter((n) => !re || re.test(n.name))
                .sort((a, b) => (me ? tilesFrom(me, a) - tilesFrom(me, b) : 0))
                .slice(0, 20);
            if (!found.length) return 'nothing nearby matches';
            return found
                .map((n) => `${pad(n.name, 22)}${pad(`(${n.x},${n.z})`, 14)}${me ? `${tilesFrom(me, n)}t` : ''}`)
                .join('\n');
        },
    },
    {
        name: 'locs',
        usage: 'locs [pattern]',
        summary: 'Nearby objects — trees, fires, doors, spots',
        needs: 'observe',
        run(args, ctx) {
            const state = requireState(ctx);
            const re = matcher(args);
            const me = state.player;
            const found = (state.nearbyLocs ?? [])
                .filter((l) => !re || re.test(l.name))
                .sort((a, b) => (me ? tilesFrom(me, a) - tilesFrom(me, b) : 0))
                .slice(0, 20);
            if (!found.length) return 'nothing nearby matches';
            return found
                .map((l) => `${pad(l.name, 22)}${pad(`(${l.x},${l.z})`, 14)}${me ? `${tilesFrom(me, l)}t` : ''}`)
                .join('\n');
        },
    },
    {
        name: 'ground',
        usage: 'ground [pattern]',
        summary: 'Items on the floor nearby',
        needs: 'observe',
        run(args, ctx) {
            const state = requireState(ctx);
            const re = matcher(args);
            const me = state.player;
            const found = (state.groundItems ?? [])
                .filter((g) => !re || re.test(g.name))
                .sort((a, b) => (me ? tilesFrom(me, a) - tilesFrom(me, b) : 0))
                .slice(0, 20);
            if (!found.length) return 'nothing on the floor nearby';
            return found
                .map((g) => `${pad(g.name, 22)}${pad(`x${g.count ?? 1}`, 6)}${pad(`(${g.x},${g.z})`, 14)}${me ? `${tilesFrom(me, g)}t` : ''}`)
                .join('\n');
        },
    },
    {
        name: 'log',
        usage: 'log [count]',
        summary: 'Recent activity the session picked up',
        needs: 'observe',
        run(args, ctx) {
            const n = Number(args[0]) || 15;
            const rows = ctx.session.log.slice(-n);
            if (!rows.length) return 'nothing logged yet';
            return rows.map((e) => `${new Date(e.at).toLocaleTimeString()}  ${e.text}`).join('\n');
        },
    },
    {
        name: 'say',
        usage: 'say <message>',
        summary: 'Talk through the bot — the one action an observer may send',
        needs: 'observe',
        async run(args, ctx) {
            const message = args.join(' ').trim();
            if (!message) return 'usage: say <message>';
            if (!ctx.say) return 'chat is not wired on this connection';
            await ctx.say(message);
            return `said: ${message}`;
        },
    },

    // ---- control: evicts whatever is driving the bot ------------------------
    {
        name: 'control',
        usage: 'control [--force]',
        summary: 'Take control — DISCONNECTS the running bot script',
        needs: 'observe',
        async run(args, ctx) {
            if (ctx.mode === 'control') return 'already in control';
            if (!ctx.takeControl) return 'this connection cannot take control';
            if (!args.includes('--force')) {
                return [
                    'Taking control disconnects whatever is currently driving this bot.',
                    'The gateway is last-controller-wins: a new control connection',
                    'evicts the existing one, and the running script does not come back',
                    'on its own. Re-run `control --force` if that is what you want.',
                ].join('\n');
            }
            await ctx.takeControl();
            return 'took control — the previous controller has been disconnected';
        },
    },
    {
        name: 'release',
        usage: 'release',
        summary: 'Drop back to read-only observing',
        needs: 'observe',
        async run(_args, ctx) {
            if (ctx.mode !== 'control') return 'not in control';
            if (!ctx.release) return 'this connection cannot release control';
            await ctx.release();
            return 'released — observing again. The bot script is still disconnected; restart it.';
        },
    },
    {
        name: 'walk',
        usage: 'walk <x> <z>',
        summary: 'Walk to a tile, handling doors on the way',
        needs: 'control',
        async run(args, ctx) {
            const x = Number(args[0]);
            const z = Number(args[1]);
            if (!Number.isFinite(x) || !Number.isFinite(z)) return 'usage: walk <x> <z>';
            if (!ctx.act) return 'no action channel on this connection';
            const from = ctx.state?.player;
            const result = await ctx.act.walkTo(x, z);
            const to = (await ctx.settle?.(2))?.player ?? ctx.state?.player;
            const moved = from && to ? tilesFrom(from, to) : 0;
            if (!result.success) return `walk failed: ${result.message ?? 'unknown'} (moved ${moved}t)`;
            const short = to ? tilesFrom(to, { x, z }) : 0;
            // `walkTo` reports success on arriving *near enough*, so say where
            // it actually stopped rather than implying the tile was reached.
            return to
                ? `at (${to.x},${to.z}) after ${moved}t` + (short > 0 ? ` — ${short}t short of the target` : '')
                : 'walk sent; no position frame came back';
        },
    },
    {
        name: 'pickpocket',
        usage: 'pickpocket [pattern] [count]',
        summary: 'Pick a pocket and report which of the twelve outcomes it was',
        needs: 'control',
        async run(args, ctx) {
            if (!ctx.act) return 'no action channel on this connection';
            const pattern = args[0] ? new RegExp(args[0], 'i') : /^(man|woman)$/i;
            const count = Math.min(Number(args[1]) || 1, 25);

            const tally = new Map<Outcome, number>();
            const lines: string[] = [];
            let coinsBefore = coinsIn(ctx.state);

            for (let attempt = 0; attempt < count; attempt++) {
                const before = ctx.state?.player?.hp ?? 0;
                const seen = ctx.state?.gameMessages?.length ?? 0;

                const event = randomEventNearby(ctx.state?.nearbyNpcs ?? []);
                if (event) {
                    lines.push(`stopped: '${event}' is a random event NPC — it can teleport the account away`);
                    tally.set('random-event', (tally.get('random-event') ?? 0) + 1);
                    break;
                }

                const result = await ctx.act.interactNpc(pattern, /pickpocket/i);
                // A failure stuns for eight ticks; wait past it before reading,
                // or the next attempt reads the previous one's messages.
                const after = await ctx.settle?.(9);
                const messages = (after?.gameMessages ?? []).slice(seen).map((m) => m.text);
                const outcome: Outcome = result.success
                    ? classify(messages, { before, after: after?.player?.hp ?? before })
                    : classify(messages.concat(result.message ?? ''), { before, after: after?.player?.hp ?? before });

                tally.set(outcome, (tally.get(outcome) ?? 0) + 1);
                if (FATAL.includes(outcome)) {
                    lines.push(`stopped on '${outcome}' — retrying will not help`);
                    break;
                }
            }

            const gained = coinsIn(ctx.state) - coinsBefore;
            const summary = [...tally.entries()].map(([o, n]) => `${o} x${n}`).join(', ');
            return [`${summary || 'nothing happened'}`, `${gained >= 0 ? '+' : ''}${num(gained)} gp`, ...lines].join('\n');
        },
    },
    {
        name: 'eat',
        usage: 'eat [food]',
        summary: 'Eat something, and say how many hitpoints it actually gave',
        needs: 'control',
        async run(args, ctx) {
            if (!ctx.act) return 'no action channel on this connection';
            const pattern = args[0] ? new RegExp(args[0], 'i') : /^shrimps$/i;
            const before = ctx.state?.player;
            if (before && before.hp >= before.maxHp) {
                return `already at ${before.hp}/${before.maxHp} — eating would waste it`;
            }
            const result = await ctx.act.eatFood(pattern);
            const after = (await ctx.settle?.(3))?.player;
            if (!result.success) return `eat failed: ${result.message ?? 'nothing matched'}`;
            if (!before || !after) return 'ate; no hitpoint frame came back';
            const healed = after.hp - before.hp;
            // Healing is capped at max, so a full-value food eaten near the top
            // returns less than its listing. Report what landed, not what it pays.
            return `${before.hp} -> ${after.hp}/${after.maxHp} hp (+${healed})`;
        },
    },
    {
        name: 'help',
        usage: 'help [command]',
        summary: 'What this console can do',
        needs: 'none',
        run(args, ctx) {
            const one = args[0] && COMMANDS.find((c) => c.name === args[0]);
            if (one) return `${one.usage}\n  ${one.summary}\n  needs: ${one.needs}`;
            const tiers: [Needs, string][] = [
                ['none', 'PLANNING — works with no game attached'],
                ['observe', 'LIVE — reads the running bot'],
                ['control', 'CONTROL — moves the character'],
            ];
            return tiers
                .map(([needs, title]) => {
                    const rows = COMMANDS.filter((c) => c.needs === needs && c.name !== 'help');
                    if (!rows.length) return '';
                    const gated = permits(ctx.mode, needs) ? '' : '  (unavailable in observe mode)';
                    // Wide enough for the longest usage line; `pad` only pads,
                    // so a short column silently runs the summary into it.
                    const width = Math.max(...COMMANDS.map((c) => c.usage.length)) + 2;
                    return `${title}${gated}\n` + rows.map((c) => `  ${pad(c.usage, width)}${c.summary}`).join('\n');
                })
                .filter(Boolean)
                .join('\n\n');
        },
    },
];

/** Split a console line into a command and its arguments. */
export function parse(line: string): { name: string; args: string[] } | null {
    const trimmed = line.trim();
    if (!trimmed) return null;
    const parts = trimmed.split(/\s+/);
    return { name: parts[0]!.toLowerCase(), args: parts.slice(1) };
}

/** Result of running one line. */
export interface CommandResult {
    ok: boolean;
    output: string;
}

/**
 * Run one console line.
 *
 * The mode gate is checked here rather than inside handlers, so a command
 * cannot forget it: a `control`-tier command in an observe connection is
 * refused before its handler ever runs.
 */
export async function execute(line: string, ctx: CommandContext): Promise<CommandResult> {
    const parsed = parse(line);
    if (!parsed) return { ok: true, output: '' };

    const command = COMMANDS.find((c) => c.name === parsed.name);
    if (!command) {
        const near = COMMANDS.map((c) => c.name).filter((n) => n.startsWith(parsed.name.slice(0, 2)));
        return {
            ok: false,
            output: `unknown command '${parsed.name}'${near.length ? ` — did you mean ${near.join(', ')}?` : ''}\ntype 'help'`,
        };
    }
    if (!permits(ctx.mode, command.needs)) {
        return {
            ok: false,
            output:
                `'${command.name}' needs ${command.needs} mode; this connection is ${ctx.mode}.\n` +
                `Run 'control --force' first — it disconnects the running bot script.`,
        };
    }
    try {
        return { ok: true, output: await command.run(parsed.args, ctx) };
    } catch (err) {
        return { ok: false, output: (err as Error).message };
    }
}

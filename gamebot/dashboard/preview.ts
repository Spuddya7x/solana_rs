/**
 * Serve the dashboard against a synthetic session, with no game attached.
 *
 * The page is the hard part to review — it only comes alive with a bot logged
 * in, which is exactly when nobody wants to be editing CSS. This drives the
 * same `SessionTracker` the real server uses with fabricated frames, so what
 * renders is the real rendering path against real-shaped data.
 *
 *   bun bots/<name>/dashboard/preview.ts            # http://localhost:8421
 *   bun bots/<name>/dashboard/preview.ts --port 9000
 */

import { join } from 'node:path';

import { xpForLevel } from '../lib/spells';
import { SessionTracker, type StateFrame } from './stats';

/**
 * Level for an XP total.
 *
 * The fixture derives levels rather than stating them, because a hand-written
 * pair that disagrees — level 9 with 1,180 XP, which is really level 10 —
 * renders as "0 XP to next level" and looks like a bug in the dashboard.
 */
function levelForXp(experience: number): number {
    let level = 1;
    while (level < 99 && xpForLevel(level + 1) <= experience) level++;
    return level;
}

/** A skill row whose level and XP cannot contradict each other. */
const skill = (name: string, experience: number, drainedTo?: number) => ({
    name,
    level: drainedTo ?? levelForXp(experience),
    baseLevel: levelForXp(experience),
    experience,
});

const args = process.argv.slice(2);
const portFlag = args.indexOf('--port');
const PORT = Number(portFlag === -1 ? '8421' : (args[portFlag + 1] ?? '8421'));

/** A clock that starts an hour and a bit back, so runtime reads plausibly. */
const START = Date.now() - 3_852_000;
let simulated = START;
const tracker = new SessionTracker(() => simulated);

function frame(over: Partial<StateFrame>): StateFrame {
    return {
        tick: 15_095,
        inGame: true,
        player: {
            name: 'mercbot01',
            combatLevel: 3,
            hp: 7,
            maxHp: 10,
            x: 3221,
            z: 3219,
            level: 0,
            runEnergy: 64,
        },
        skills: [],
        inventory: [],
        ...over,
    };
}

// A thieving run partway through its second forage trip: Thieving carried to
// 14, Fishing and Cooking dragged along by the food loop, Firemaking and
// Woodcutting picking up scraps from lighting fires.
const skillsAt = (thievingXp: number, fishXp: number, cookXp: number, hp: number) => [
    skill('Thieving', thievingXp),
    skill('Fishing', fishXp),
    skill('Cooking', cookXp),
    skill('Firemaking', 420),
    skill('Woodcutting', 275),
    // Hitpoints start at 10 and are drained by every failed pickpocket, which
    // is the whole reason the true level has to come from `baseLevel`.
    skill('Hitpoints', 1_154, hp),
    skill('Attack', 0),
    skill('Magic', 0),
];

const inventory = [
    { slot: 0, id: 995, name: 'Coins', count: 4_312 },
    { slot: 1, id: 315, name: 'Shrimps', count: 1 },
    { slot: 2, id: 315, name: 'Shrimps', count: 1 },
    { slot: 3, id: 315, name: 'Shrimps', count: 1 },
    { slot: 4, id: 315, name: 'Shrimps', count: 1 },
    { slot: 5, id: 315, name: 'Shrimps', count: 1 },
    { slot: 6, id: 315, name: 'Shrimps', count: 1 },
    { slot: 7, id: 315, name: 'Shrimps', count: 1 },
    { slot: 8, id: 315, name: 'Shrimps', count: 1 },
    { slot: 9, id: 315, name: 'Shrimps', count: 1 },
    { slot: 12, id: 303, name: 'Small fishing net', count: 1 },
    { slot: 13, id: 590, name: 'Tinderbox', count: 1 },
    { slot: 14, id: 1351, name: 'Bronze axe', count: 1 },
    { slot: 15, id: 1511, name: 'Logs', count: 1 },
];

// Seed the baseline an hour ago, then walk forward so the rates are real
// arithmetic over a real elapsed time rather than numbers typed into a mock.
tracker.push(frame({ skills: skillsAt(0, 0, 0, 10), inventory: [] }));
simulated += 3_852_000;
tracker.push(frame({ skills: skillsAt(2_310, 1_180, 640, 7), inventory }));

for (const [text, tick] of [
    ['You attempt to light the logs.', 15_020],
    ['The fire catches and the logs begin to burn.', 15_030],
    ['The shrimps are now nicely cooked.', 15_042],
    ['You accidentally burn the shrimps.', 15_050],
    ["You pick the man's pocket.", 15_070],
    ['You fail to pick the mans pocket.', 15_078],
    ["You pick the man's pocket.", 15_086],
    ["You pick the man's pocket.", 15_094],
] as const) {
    tracker.push(
        frame({ skills: skillsAt(2_310, 1_180, 640, 7), inventory, gameMessages: [{ type: 0, text, sender: '', tick }] }),
    );
}

const PAGE = join(import.meta.dir, 'index.html');
const clients = new Set<{ send: (data: string) => void }>();

const server = Bun.serve({
    port: PORT,
    hostname: '127.0.0.1',
    fetch(request, srv) {
        const url = new URL(request.url);
        if (url.pathname === '/ws') {
            return srv.upgrade(request) ? undefined : new Response('upgrade failed', { status: 400 });
        }
        if (url.pathname === '/session') return Response.json(tracker.snapshot());
        return new Response(Bun.file(PAGE), { headers: { 'content-type': 'text/html; charset=utf-8' } });
    },
    websocket: {
        open(ws) {
            clients.add(ws);
            ws.send(JSON.stringify(tracker.snapshot()));
        },
        close(ws) {
            clients.delete(ws);
        },
        message() {},
    },
});

console.log(`[preview] http://localhost:${server.port} (synthetic session, no game attached)`);

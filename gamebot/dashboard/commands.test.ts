import { describe, expect, test } from 'bun:test';

import { COMMANDS, execute, parse, permits, type CommandContext, type Needs } from './commands';
import { SessionTracker } from './stats';

function context(over: Partial<CommandContext> = {}): CommandContext {
    const tracker = new SessionTracker(() => 1_000_000);
    tracker.push({
        tick: 100,
        inGame: true,
        player: { name: 'testbot', combatLevel: 3, hp: 7, maxHp: 10, x: 3222, z: 3218, level: 0, runEnergy: 90 },
        skills: [
            { name: 'Thieving', level: 12, baseLevel: 12, experience: 1_800 },
            { name: 'Cooking', level: 1, baseLevel: 1, experience: 0 },
        ],
        inventory: [{ slot: 0, id: 995, name: 'Coins', count: 1_500 }],
    });
    return { session: tracker.snapshot(), state: null, mode: 'observe', ...over };
}

describe('the mode gate', () => {
    test('planning runs from anywhere, including with nothing attached', () => {
        for (const mode of ['none', 'observe', 'control'] as Needs[]) {
            expect(permits(mode, 'none')).toBe(true);
        }
    });

    test('control implies observe, but not the reverse', () => {
        expect(permits('control', 'observe')).toBe(true);
        expect(permits('observe', 'control')).toBe(false);
        expect(permits('none', 'observe')).toBe(false);
    });

    test('an observer is refused a control command before the handler runs', async () => {
        // The gate has to sit in the dispatcher: a handler that forgot to check
        // would evict the running bot script on its first call.
        let ran = false;
        const command = {
            name: 'boom',
            usage: 'boom',
            summary: 'test',
            needs: 'control' as Needs,
            run: () => {
                ran = true;
                return 'ran';
            },
        };
        COMMANDS.push(command);
        try {
            const result = await execute('boom', context({ mode: 'observe' }));
            expect(result.ok).toBe(false);
            expect(ran).toBe(false);
            expect(result.output).toContain('control');
        } finally {
            COMMANDS.pop();
        }
    });
});

describe('taking control', () => {
    test('refuses without --force, and does not escalate', async () => {
        let took = false;
        const result = await execute('control', context({ takeControl: async () => void (took = true) }));
        expect(took).toBe(false);
        expect(result.output).toContain('disconnects');
    });

    test('--force escalates, and says what it broke', async () => {
        let took = false;
        const result = await execute(
            'control --force',
            context({ takeControl: async () => void (took = true) }),
        );
        expect(took).toBe(true);
        expect(result.output).toContain('disconnected');
    });

    test('is a no-op when already in control', async () => {
        let took = false;
        const result = await execute(
            'control --force',
            context({ mode: 'control', takeControl: async () => void (took = true) }),
        );
        expect(took).toBe(false);
        expect(result.output).toBe('already in control');
    });

    test('releasing says the script is still gone', async () => {
        // Releasing gives up control; it does not bring the evicted script back.
        const result = await execute('release', context({ mode: 'control', release: async () => {} }));
        expect(result.output).toContain('restart');
    });
});

describe('parsing', () => {
    test('splits on whitespace and lowercases the verb', () => {
        expect(parse('  SHOP  lobster 150 ')).toEqual({ name: 'shop', args: ['lobster', '150'] });
    });

    test('an empty line is not an error', async () => {
        expect(parse('   ')).toBeNull();
        expect(await execute('   ', context())).toEqual({ ok: true, output: '' });
    });

    test('an unknown command suggests near misses', async () => {
        const result = await execute('shp lobster', context());
        expect(result.ok).toBe(false);
        expect(result.output).toContain('shop');
    });
});

describe('planning commands work with no game attached', () => {
    const detached = () => context({ mode: 'none', state: null });

    test('thieve prints the table at an explicit level', async () => {
        const result = await execute('thieve 10', detached());
        expect(result.ok).toBe(true);
        expect(result.output).toContain('farmer');
        expect(result.output).toContain('locked'); // the warrior woman, at level 25
    });

    test('forage sizes a trip and names the circuit', async () => {
        const result = await execute('forage 12', detached());
        expect(result.output).toMatch(/cooked shrimp/);
        expect(result.output).toContain('3267');
    });

    test('shop names the specialist when one is reachable', async () => {
        const result = await execute('shop uncut_diamond 3200', detached());
        expect(result.ok).toBe(true);
        expect(result.output).toContain('specialist:');
    });

    test('shop says so when the only buyer is a general store', async () => {
        // A general store buys anything, so every item finds *a* buyer at
        // 400/1000. That is low-alchemy value against a 36% floor and gone by
        // the third unit, so reporting it as a find would be misleading.
        const result = await execute('shop lobster 150', detached());
        expect(result.output).toContain('no reachable specialist');
        expect(result.output).toContain('General Store');
    });

    test('shop needs an item', async () => {
        expect((await execute('shop', detached())).output).toContain('usage:');
    });
});

describe('live commands', () => {
    test('read the derived session, not raw state', async () => {
        const result = await execute('where', context());
        expect(result.output).toContain('3222');
        expect(result.output).toContain('7/10 hp');
    });

    test('skills shows the drained value beside the true one', async () => {
        const ctx = context();
        // Hitpoints drained is the case that matters; use a session that has it.
        const tracker = new SessionTracker(() => 1_000_000);
        tracker.push({
            tick: 1,
            inGame: true,
            player: null,
            skills: [{ name: 'Hitpoints', level: 4, baseLevel: 10, experience: 1_154 }],
            inventory: [],
        });
        const result = await execute('skills', { ...ctx, session: tracker.snapshot() });
        expect(result.output).toContain('10 (4)');
    });

    test('a scan without world state explains itself rather than throwing', async () => {
        const result = await execute('npcs man', context({ state: null }));
        expect(result.ok).toBe(false);
        expect(result.output).toContain('logged in');
    });

    test('npcs sorts by distance from the player', async () => {
        const result = await execute(
            'npcs',
            context({
                state: {
                    tick: 1,
                    player: { name: 'b', x: 3222, z: 3218, level: 0, hp: 10, maxHp: 10 },
                    nearbyNpcs: [
                        { name: 'Far guard', x: 3260, z: 3218 },
                        { name: 'Near man', x: 3223, z: 3218 },
                    ],
                },
            }),
        );
        expect(result.output.indexOf('Near man')).toBeLessThan(result.output.indexOf('Far guard'));
    });

    test('say goes through the observer channel', async () => {
        const sent: string[] = [];
        const result = await execute('say hello there', context({ say: async (m) => void sent.push(m) }));
        expect(sent).toEqual(['hello there']);
        expect(result.ok).toBe(true);
    });

    test('say with no message does not send an empty line', async () => {
        const sent: string[] = [];
        await execute('say', context({ say: async (m) => void sent.push(m) }));
        expect(sent).toEqual([]);
    });
});

describe('help', () => {
    test('groups by tier and flags what the mode cannot reach', async () => {
        const result = await execute('help', context({ mode: 'observe' }));
        expect(result.output).toContain('PLANNING');
        expect(result.output).toContain('LIVE');
    });

    test('explains a single command', async () => {
        const result = await execute('help shop', context());
        expect(result.output).toContain('shop <item>');
        expect(result.output).toContain('needs: none');
    });

    test('every command is documented and uniquely named', () => {
        const names = COMMANDS.map((c) => c.name);
        expect(new Set(names).size).toBe(names.length);
        for (const command of COMMANDS) {
            expect(command.usage.startsWith(command.name)).toBe(true);
            expect(command.summary.length).toBeGreaterThan(8);
        }
    });
});

describe('control-tier actions', () => {
    /** A fake action channel plus the frames it produces, so every branch is reachable. */
    function acting(over: {
        walk?: () => { success: boolean; message?: string };
        interact?: () => { success: boolean; message?: string };
        eat?: () => { success: boolean; message?: string };
        frames?: Partial<import('./commands').Worldish>[];
        start?: Partial<import('./commands').Worldish>;
    }) {
        const base = {
            tick: 1,
            player: { name: 'b', x: 3222, z: 3218, level: 0, hp: 10, maxHp: 10 },
            nearbyNpcs: [{ name: 'Man', x: 3221, z: 3219 }],
            gameMessages: [] as { text: string }[],
            inventory: [{ name: 'Coins', count: 100 }],
        };
        let current = { ...base, ...over.start } as import('./commands').Worldish;
        const queue = [...(over.frames ?? [])];
        const calls: string[] = [];
        const ctx = context({
            mode: 'control',
            state: current,
            settle: async () => {
                const next = queue.shift();
                if (next) current = { ...current, ...next } as import('./commands').Worldish;
                ctx.state = current;
                return current;
            },
            act: {
                walkTo: async (x, z) => (calls.push(`walk ${x} ${z}`), over.walk?.() ?? { success: true }),
                interactNpc: async () => (calls.push('interact'), over.interact?.() ?? { success: true }),
                eatFood: async () => (calls.push('eat'), over.eat?.() ?? { success: true }),
            },
        });
        return { ctx, calls };
    }

    test('walk reports where it actually stopped, not where it was aimed', async () => {
        // walkTo succeeds on arriving near enough, so saying "arrived" would lie.
        const { ctx } = acting({
            frames: [{ player: { name: 'b', x: 3230, z: 3218, level: 0, hp: 10, maxHp: 10 } }],
        });
        const result = await execute('walk 3232 3218', ctx);
        expect(result.output).toContain('(3230,3218)');
        expect(result.output).toContain('2t short');
    });

    test('walk rejects a non-numeric tile before sending anything', async () => {
        const { ctx, calls } = acting({});
        expect((await execute('walk here', ctx)).output).toContain('usage:');
        expect(calls).toEqual([]);
    });

    test('walk surfaces a failure with the distance covered', async () => {
        const { ctx } = acting({ walk: () => ({ success: false, message: 'blocked' }) });
        const result = await execute('walk 3300 3300', ctx);
        expect(result.output).toContain('blocked');
    });

    test('pickpocket names the outcome rather than just counting coins', async () => {
        const { ctx } = acting({
            frames: [{ gameMessages: [{ text: "You pick the man's pocket." }] }],
        });
        const result = await execute('pickpocket man 1', ctx);
        expect(result.output).toContain('success x1');
    });

    test('pickpocket stops on a fatal outcome instead of hammering', async () => {
        const { ctx, calls } = acting({
            frames: [
                { gameMessages: [{ text: 'You need level 10 thieving to pick the pocket.' }] },
                { gameMessages: [{ text: 'You need level 10 thieving to pick the pocket.' }] },
            ],
        });
        const result = await execute('pickpocket farmer 5', ctx);
        expect(result.output).toContain('level-too-low');
        expect(result.output).toContain('retrying will not help');
        expect(calls.filter((c) => c === 'interact')).toHaveLength(1);
    });

    test('pickpocket refuses to start next to a random event NPC', async () => {
        // The maze and cube events teleport the account away; walking into one
        // unattended ruins the run.
        const { ctx, calls } = acting({
            start: { nearbyNpcs: [{ name: 'Mysterious old man', x: 3222, z: 3218 }] },
        });
        const result = await execute('pickpocket man 5', ctx);
        expect(result.output).toContain('random event');
        expect(calls).toEqual([]);
    });

    test('pickpocket caps the count so one line cannot run forever', async () => {
        const { ctx, calls } = acting({
            frames: Array.from({ length: 40 }, () => ({ gameMessages: [{ text: "You pick the man's pocket." }] })),
        });
        await execute('pickpocket man 999', ctx);
        expect(calls.filter((c) => c === 'interact').length).toBeLessThanOrEqual(25);
    });

    test('eat reports the hitpoints that landed, not the food listing', async () => {
        // Healing clips at max, so a 3 hp shrimp eaten at 9/10 gives 1.
        const { ctx } = acting({
            start: { player: { name: 'b', x: 0, z: 0, level: 0, hp: 9, maxHp: 10 } },
            frames: [{ player: { name: 'b', x: 0, z: 0, level: 0, hp: 10, maxHp: 10 } }],
        });
        const result = await execute('eat shrimps', ctx);
        expect(result.output).toContain('9 -> 10/10 hp (+1)');
    });

    test('eat refuses at full hitpoints rather than wasting the food', async () => {
        const { ctx, calls } = acting({});
        const result = await execute('eat', ctx);
        expect(result.output).toContain('would waste it');
        expect(calls).toEqual([]);
    });

    test('eat surfaces having nothing to eat', async () => {
        const { ctx } = acting({
            start: { player: { name: 'b', x: 0, z: 0, level: 0, hp: 4, maxHp: 10 } },
            eat: () => ({ success: false, message: 'no shrimps in inventory' }),
        });
        expect((await execute('eat shrimps', ctx)).output).toContain('no shrimps');
    });

    test('none of the three run without an action channel', async () => {
        for (const line of ['walk 1 1', 'pickpocket', 'eat']) {
            const result = await execute(line, context({ mode: 'control' }));
            expect(result.output).toContain('no action channel');
        }
    });

    test('none of the three run from an observer', async () => {
        for (const line of ['walk 1 1', 'pickpocket', 'eat']) {
            const result = await execute(line, context({ mode: 'observe' }));
            expect(result.ok).toBe(false);
            expect(result.output).toContain('needs control');
        }
    });
});

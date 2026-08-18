import { describe, expect, test } from 'bun:test';

import {
    INVENTORY_SLOTS,
    SessionTracker,
    countCoins,
    formatCount,
    formatRuntime,
    layOutInventory,
    type StateFrame,
} from './stats';

/** A clock the tests drive by hand, so rates are exact rather than flaky. */
function clock(start = 1_000_000) {
    let t = start;
    return {
        now: () => t,
        advance(seconds: number) {
            t += seconds * 1000;
        },
    };
}

function frame(over: Partial<StateFrame> = {}): StateFrame {
    return {
        tick: 100,
        inGame: true,
        player: {
            name: 'testbot',
            combatLevel: 3,
            hp: 10,
            maxHp: 10,
            x: 3222,
            z: 3218,
            level: 0,
            runEnergy: 100,
        },
        skills: [{ name: 'Thieving', level: 1, baseLevel: 1, experience: 0 }],
        inventory: [],
        ...over,
    };
}

describe('coins', () => {
    test('are counted as a stack, not as slots', () => {
        expect(countCoins([{ name: 'Coins', count: 4_312 }])).toBe(4_312);
    });

    test('ignore anything that merely mentions coins', () => {
        // `coin pouch` and `Dwarven coin` would both be wrong to count.
        expect(countCoins([{ name: 'Coin pouch', count: 5 }])).toBe(0);
    });
});

describe('the inventory grid', () => {
    test('is always 28 slots, gaps included', () => {
        const grid = layOutInventory([{ slot: 3, id: 995, name: 'Coins', count: 12 }]);
        expect(grid).toHaveLength(INVENTORY_SLOTS);
        expect(grid[0]).toBeNull();
        expect(grid[3]?.name).toBe('Coins');
    });

    test('drops slots the game should never send', () => {
        const grid = layOutInventory([
            { slot: 28, id: 1, name: 'Out of range', count: 1 },
            { slot: -1, id: 2, name: 'Negative', count: 1 },
        ]);
        expect(grid.every((s) => s === null)).toBe(true);
    });
});

describe('session rates', () => {
    test('xp per hour is the session average, not an instant rate', () => {
        const c = clock();
        const tracker = new SessionTracker(c.now);
        tracker.push(frame());
        c.advance(1800); // half an hour
        tracker.push(frame({ skills: [{ name: 'Thieving', level: 5, baseLevel: 5, experience: 400 }] }));

        const skill = tracker.snapshot().skills.find((s) => s.name === 'Thieving')!;
        expect(skill.gained).toBe(400);
        expect(skill.perHour).toBeCloseTo(800, 5);
    });

    test('the first frame is a baseline, so it never counts as progress', () => {
        const c = clock();
        const tracker = new SessionTracker(c.now);
        // Attaching to a bot that has been running for hours must not credit
        // the session with its existing 50,000 xp.
        tracker.push(frame({ skills: [{ name: 'Thieving', level: 40, baseLevel: 40, experience: 50_000 }] }));
        c.advance(60);

        const snapshot = tracker.snapshot();
        expect(snapshot.totalXpGained).toBe(0);
        expect(snapshot.skills[0]?.startLevel).toBe(40);
    });

    test('rates are zero rather than infinite before any time has passed', () => {
        const tracker = new SessionTracker(clock().now);
        tracker.push(frame());
        const snapshot = tracker.snapshot();
        expect(snapshot.totalXpPerHour).toBe(0);
        expect(snapshot.coinsPerHour).toBe(0);
        expect(Number.isFinite(snapshot.coinsPerHour)).toBe(true);
    });

    test('coins carried can go down, and the report says so', () => {
        // Selling, dropping or dying all move this. It is a carried total, not
        // a profit figure, and it must not clamp at zero.
        const c = clock();
        const tracker = new SessionTracker(c.now);
        tracker.push(frame({ inventory: [{ slot: 0, id: 995, name: 'Coins', count: 5_000 }] }));
        c.advance(3600);
        tracker.push(frame({ inventory: [{ slot: 0, id: 995, name: 'Coins', count: 1_000 }] }));

        const snapshot = tracker.snapshot();
        expect(snapshot.coins).toBe(1_000);
        expect(snapshot.coinsGained).toBe(-4_000);
        expect(snapshot.coinsPerHour).toBeCloseTo(-4_000, 5);
    });

    test('time to level uses the measured rate', () => {
        const c = clock();
        const tracker = new SessionTracker(c.now);
        tracker.push(frame());
        c.advance(3600);
        // Level 5 is 388 xp and level 6 is 512, so 400 xp leaves 112 to go. An
        // hour bought 400 xp, so the remainder is about a sixth of an hour.
        tracker.push(frame({ skills: [{ name: 'Thieving', level: 5, baseLevel: 5, experience: 400 }] }));

        const skill = tracker.snapshot().skills.find((s) => s.name === 'Thieving')!;
        expect(skill.toNextLevel).toBe(112);
        expect(skill.secondsToLevel).toBeCloseTo((112 / 400) * 3600, 0);
    });

    test('a skill that is not moving has no ETA', () => {
        const c = clock();
        const tracker = new SessionTracker(c.now);
        tracker.push(frame());
        c.advance(3600);
        tracker.push(frame());
        expect(tracker.snapshot().skills[0]?.secondsToLevel).toBeNull();
    });
});

describe('boosted and drained levels', () => {
    test('the true level comes from baseLevel, not the drained one', () => {
        // A thieving bot lives at reduced hitpoints: every failed pickpocket
        // takes one and they come back at one a minute. Reading `level` would
        // report the account as lower-levelled than it is.
        const tracker = new SessionTracker(clock().now);
        tracker.push(
            frame({ skills: [{ name: 'Hitpoints', level: 7, baseLevel: 10, experience: 1_358 }] }),
        );
        const hp = tracker.snapshot().skills[0]!;
        expect(hp.level).toBe(10);
        expect(hp.current).toBe(7);
    });

    test('the next-level target is measured against the true level', () => {
        // Level 11 is 1358 xp. Drained to 7, the old code aimed at level 8 —
        // a target the account passed long ago — and reported it as reached.
        const tracker = new SessionTracker(clock().now);
        tracker.push(
            frame({ skills: [{ name: 'Hitpoints', level: 7, baseLevel: 10, experience: 1_200 }] }),
        );
        expect(tracker.snapshot().skills[0]!.toNextLevel).toBe(158);
    });

    test('healing back up is not announced as a level-up', () => {
        // Otherwise every shrimp the thief eats logs a fake congratulation.
        const tracker = new SessionTracker(clock().now);
        tracker.push(frame({ skills: [{ name: 'Hitpoints', level: 3, baseLevel: 10, experience: 1_358 }] }));
        tracker.push(frame({ skills: [{ name: 'Hitpoints', level: 6, baseLevel: 10, experience: 1_358 }] }));
        tracker.push(frame({ skills: [{ name: 'Hitpoints', level: 10, baseLevel: 10, experience: 1_358 }] }));
        expect(tracker.snapshot().log).toHaveLength(0);
    });

    test('a real level-up still registers while drained', () => {
        const tracker = new SessionTracker(clock().now);
        tracker.push(frame({ skills: [{ name: 'Hitpoints', level: 4, baseLevel: 10, experience: 1_358 }] }));
        tracker.push(frame({ skills: [{ name: 'Hitpoints', level: 4, baseLevel: 11, experience: 1_600 }] }));
        expect(tracker.snapshot().log.at(-1)?.text).toBe('Hitpoints level 11');
    });
});

describe('the activity log', () => {
    test('reads the activity out of the game\'s own messages', () => {
        const tracker = new SessionTracker(clock().now);
        tracker.push(
            frame({
                gameMessages: [
                    { type: 0, text: "You pick the man's pocket.", sender: '', tick: 101 },
                ],
            }),
        );
        expect(tracker.snapshot().activity).toBe('Thieving');
    });

    test('a failed pickpocket reads as stunned, not as success', () => {
        const tracker = new SessionTracker(clock().now);
        tracker.push(
            frame({
                gameMessages: [
                    { type: 0, text: 'You fail to pick the mans pocket.', sender: '', tick: 102 },
                ],
            }),
        );
        const snapshot = tracker.snapshot();
        expect(snapshot.activity).toBe('Thieving (stunned)');
        expect(snapshot.log.at(-1)?.kind).toBe('damage');
    });

    test('the same message on a later frame is not logged twice', () => {
        // The feed is a rolling window: every frame re-sends recent messages.
        const tracker = new SessionTracker(clock().now);
        const messages = [{ type: 0, text: "You pick the man's pocket.", sender: '', tick: 101 }];
        tracker.push(frame({ gameMessages: messages }));
        tracker.push(frame({ gameMessages: messages }));
        tracker.push(frame({ gameMessages: messages }));
        expect(tracker.snapshot().log).toHaveLength(1);
    });

    test('level-ups are logged from the level, not from the message', () => {
        // A busy tick can push the game's congratulation out of the window; the
        // level itself cannot be missed.
        const tracker = new SessionTracker(clock().now);
        tracker.push(frame());
        tracker.push(frame({ skills: [{ name: 'Thieving', level: 2, baseLevel: 2, experience: 90 }] }));

        const entry = tracker.snapshot().log.at(-1);
        expect(entry?.kind).toBe('levelup');
        expect(entry?.text).toBe('Thieving level 2');
    });

    test('a skill seen for the first time is not a level-up', () => {
        const tracker = new SessionTracker(clock().now);
        tracker.push(frame({ skills: [{ name: 'Thieving', level: 40, baseLevel: 40, experience: 50_000 }] }));
        expect(tracker.snapshot().log).toHaveLength(0);
    });
});

describe('formatting', () => {
    test('runtime drops the hours until there are some', () => {
        expect(formatRuntime(65)).toBe('1m 05s');
        expect(formatRuntime(3_852)).toBe('1h 04m 12s');
        expect(formatRuntime(-5)).toBe('0m 00s');
    });

    test('counts compact the way a bot paint does', () => {
        expect(formatCount(847)).toBe('847');
        expect(formatCount(1_240)).toBe('1.2K');
        expect(formatCount(12_300)).toBe('12K');
        expect(formatCount(1_400_000)).toBe('1.4M');
        expect(formatCount(-4_000)).toBe('-4.0K');
    });
});

describe('a session with no frames yet', () => {
    test('renders without throwing', () => {
        const snapshot = new SessionTracker(clock().now).snapshot();
        expect(snapshot.name).toBeNull();
        expect(snapshot.inGame).toBe(false);
        expect(snapshot.inventory).toHaveLength(INVENTORY_SLOTS);
        expect(snapshot.skills).toEqual([]);
    });
});

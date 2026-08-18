/**
 * Get a fresh account off Tutorial Island.
 *
 * A new character spawns at (6976, 6464) with an empty inventory, and nothing
 * else in this directory works until it is out: no tools, and no `Man` or
 * `Woman` to pick. `tutorial_complete` then grants the bronze axe, tinderbox
 * and small fishing net the forage loop needs.
 *
 * The skip is offered only when `map_live = false` — a dev world. There is no
 * automation for the real tutorial.
 *
 *   bun bots/<name>/tools/skip-tutorial.ts [botname]
 */
import { BotActions } from '../../../sdk/actions';
import { BotSDK } from '../../../sdk/index';

const sdk = new BotSDK({
    botUsername: process.argv[2] ?? process.env.BOT_USERNAME ?? 'mercbot01',
    password: process.env.PASSWORD ?? 'test',
    gatewayUrl: process.env.GATEWAY_URL ?? 'ws://localhost:7780',
    connectionMode: 'control',
    autoLaunchBrowser: false,
});
await sdk.connect();
await sdk.waitForCondition((s) => s.inGame && !!s.player, 30_000);
const before = sdk.getState()?.player;
console.log(`before: (${before?.worldX},${before?.worldZ})`);

const result = await new BotActions(sdk).skipTutorial();

// An account that is already out has no tutorial NPC to talk to, which is a
// success for our purposes and not the failure the SDK reports it as.
if (!result.success && /no tutorial npc/i.test(result.message ?? '')) {
    const kit = sdk.getInventory().filter(Boolean).map((i) => i!.name);
    console.log(`already out of the tutorial — at (${before?.worldX},${before?.worldZ})`);
    console.log(`kit:    ${kit.join(', ') || '(empty)'}`);
    process.exit(0);
}
console.log(result.message ?? JSON.stringify(result));
if (!result.success) process.exit(1);

await sdk.waitForTicks(5);
const after = sdk.getState()?.player;
const kit = sdk.getInventory().filter(Boolean).map((i) => i!.name);
console.log(`after:  (${after?.worldX},${after?.worldZ})`);
console.log(`kit:    ${kit.join(', ') || '(empty)'}`);
process.exit(0);

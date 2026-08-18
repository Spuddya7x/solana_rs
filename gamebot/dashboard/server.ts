/**
 * A window that watches a bot without touching it.
 *
 * The gateway's SDK protocol has two connection modes, and the second one is
 * what makes this possible at all:
 *
 * * `control` — full actions, and **connecting pre-empts any existing
 *   controller**. Two control connections fight; the newcomer wins.
 * * `observe` — read-only state and chat plus the `say` action. It never
 *   pre-empts and is never pre-empted, and multiple observers coexist freely.
 *
 * So this attaches as an observer and gets the same `BotWorldState` frames the
 * controller sees, with no coupling to the running script and no risk of
 * stealing its session. It works against any of the bots in this directory —
 * `thief.ts`, `moneymaker.ts`, `campaign.ts` — and against one that was already
 * running before the dashboard started. Closing the window changes nothing.
 *
 * The observer's read-only-ness is the gateway's rule, not a convention here:
 * `sdk_action` from an observe-mode session is rejected unless it is `say`.
 *
 * Everything on the page is derived from those frames by `stats.ts`. The bot
 * scripts report nothing and do not know this exists.
 *
 *   bun bots/<name>/dashboard/server.ts            # creds from bots/<name>/bot.env
 *   bun bots/<name>/dashboard/server.ts --bot mybot --port 842
 *   bun bots/<name>/dashboard/server.ts --server localhost
 */

import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { BotSDK, deriveGatewayUrl } from '../../../sdk/index';
import { SessionTracker, type StateFrame } from './stats';

const args = process.argv.slice(2);
const flag = (name: string, fallback = '') => {
    const i = args.indexOf(`--${name}`);
    return i === -1 ? fallback : (args[i + 1] ?? fallback);
};

/** The bot directory this dashboard ships inside, for `bot.env` resolution. */
const BOT_DIR = join(import.meta.dir, '..');
const BOT_NAME = flag('bot') || BOT_DIR.split('/').pop() || 'mercbot';
const PORT = Number(flag('port', '8420'));
/** How often to push a snapshot to the page. Frames arrive faster than this;
 * a tick is 600ms and re-rendering on every one is wasted work. */
const PUSH_MS = Number(flag('interval', '1000'));

/** Same `bot.env` resolution as `sdk/cli.ts` and `sdk/chat.ts`. */
function loadBotEnv(botName: string): { username: string; password: string; server?: string } | null {
    const envPath = join(process.cwd(), 'bots', botName, 'bot.env');
    if (!existsSync(envPath)) return null;
    const env: Record<string, string> = {};
    for (const line of readFileSync(envPath, 'utf-8').split('\n')) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('#')) continue;
        const eq = trimmed.indexOf('=');
        if (eq > 0) env[trimmed.slice(0, eq).trim()] = trimmed.slice(eq + 1).trim();
    }
    if (!env.BOT_USERNAME || !env.PASSWORD) return null;
    return { username: env.BOT_USERNAME, password: env.PASSWORD, server: env.SERVER };
}

const botEnv = loadBotEnv(BOT_NAME);
const username = flag('username') || botEnv?.username || process.env.USERNAME || BOT_NAME;
const password = flag('password') || botEnv?.password || process.env.PASSWORD || '';
const server = flag('server') || botEnv?.server || process.env.SERVER || 'rs-sdk-demo.fly.dev';
const isLocal = server === 'localhost' || server.startsWith('localhost:');

if (!password && !isLocal) {
    console.error(`Error: no password for '${username}'.`);
    console.error(`  Put BOT_USERNAME/PASSWORD in bots/${BOT_NAME}/bot.env, or pass --password.`);
    process.exit(1);
}

const tracker = new SessionTracker();
const clients = new Set<{ send: (data: string) => void }>();

const sdk = new BotSDK({
    botUsername: username,
    password,
    gatewayUrl: deriveGatewayUrl(server),
    // The whole point: watch without stealing the session from the bot script.
    connectionMode: 'observe',
    autoReconnect: true,
    // The bot script owns the game client; the dashboard must never open one.
    autoLaunchBrowser: false,
    // Attaching to a bot that is not logged in yet is normal — the dashboard is
    // often started first. Don't block on a ready state that may be minutes off.
    readyTimeout: 0,
    showChat: false,
});

sdk.onStateUpdate((state) => {
    tracker.push(state as unknown as StateFrame);
});

sdk.onConnectionStateChange((connection, attempt) => {
    const note = attempt ? `${connection} (attempt ${attempt})` : connection;
    console.log(`[dashboard] gateway ${note}`);
    tracker.append(`gateway ${note}`, 'system');
});

console.log(`[dashboard] observing '${username}' via ${server}`);
try {
    await sdk.connect();
    console.log('[dashboard] attached as observer (the bot is untouched)');
} catch (err) {
    // Not fatal: autoReconnect keeps trying, and the page renders "waiting".
    console.error(`[dashboard] could not reach the gateway: ${(err as Error).message}`);
    console.error('[dashboard] retrying in the background; the page will fill in when it connects');
}

/** Broadcast a snapshot to every open window. */
function broadcast(): void {
    if (clients.size === 0) return;
    const payload = JSON.stringify(tracker.snapshot());
    for (const client of clients) {
        try {
            client.send(payload);
        } catch {
            clients.delete(client);
        }
    }
}
setInterval(broadcast, PUSH_MS);

const PAGE = join(import.meta.dir, 'index.html');

const httpServer = Bun.serve({
    port: PORT,
    // Bound to loopback on purpose: the page carries no authentication and the
    // observer socket behind it is already authenticated as the bot.
    hostname: '127.0.0.1',
    fetch(request, srv) {
        const url = new URL(request.url);
        if (url.pathname === '/ws') {
            return srv.upgrade(request) ? undefined : new Response('upgrade failed', { status: 400 });
        }
        if (url.pathname === '/session') {
            return Response.json(tracker.snapshot());
        }
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
        message() {
            // The page is a display. It sends nothing, and nothing it could send
            // would be honoured — an observer connection cannot act anyway.
        },
    },
});

console.log(`[dashboard] http://localhost:${httpServer.port}`);

for (const signal of ['SIGINT', 'SIGTERM'] as const) {
    process.on(signal, () => {
        console.log('\n[dashboard] detaching');
        sdk.disconnect();
        httpServer.stop();
        process.exit(0);
    });
}

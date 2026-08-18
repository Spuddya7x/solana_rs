# Running it locally

Everything here was executed against a real local server before being written
down. Where something is *not* verified, it says so.

## The two repos

```sh
# 1. The game: server, SDK, gateway. This is the thing that runs.
git clone https://github.com/MidTermDev/mercantile
cd mercantile

# 2. This work: the Rust bot and the game bot.
git clone -b claude/framework-trading-bot-xk8oz3 \
    https://github.com/Spuddya7x/solana_rs ../solana_rs

# The game bot is designed to live inside mercantile's bots/ directory.
cp -r ../solana_rs/gamebot bots/mercbot
```

`bots/` is where the SDK's own tooling looks for a bot (`bots/<name>/bot.env`),
which is why it goes there rather than being run from `solana_rs`.

## Four terminals

### 1 — the game engine

```sh
cd mercantile/server/engine
bun install
BUILD_VERIFY=false bun run src/app.ts
```

First boot packs the cache and takes a couple of minutes. Wait for:

```
World ready: Visit http://localhost:8888/rs2.cgi
```

`BUILD_VERIFY=false` is needed because mercantile patches the content and the
`.npc` checksum no longer matches upstream's. The engine tells you this itself
and names the flag.

### 2 — the gateway

```sh
cd mercantile/server/gateway
bun install
cd ../.. && bun server/gateway/gateway.ts
```

```
[Gateway] Gateway running at http://localhost:7780
[Gateway] Authentication DISABLED (set LOGIN_SERVER=true to enable)
```

Auth disabled is the local default — any username/password works.

### 3 — a game client for the bot to drive

Create `bots/mercbot/bot.env` first:

```ini
BOT_USERNAME=mercbot01
PASSWORD=test
SERVER=localhost:8888
GATEWAY_URL=ws://localhost:7780
PROFANITY_FILTER=false
```

`GATEWAY_URL` is not optional here. The runner otherwise derives the gateway
from `SERVER`, and `localhost:8888` is the engine, not the gateway.

Then the **headless** client — no browser needed:

```sh
bun server/webclient/src/lite/runner.ts mercbot
```

```
[lite-runner] 'mercbot01' logged into localhost:8888
[lite-runner] Gateway connected, registering as 'mercbot01'
```

To *watch* it instead, open `http://localhost:8888/bot?bot=mercbot01&password=test`
in a browser — but run one or the other, not both. A second client takes the
session over (the engine logs `session takeover`).

### 4 — the dashboard

```sh
bun bots/mercbot/dashboard/server.ts --server localhost:8888
```

Open <http://localhost:8420>. It attaches as an **observer**, so it never
disturbs whatever is driving the bot.

## The first thing to do: skip the tutorial

A fresh account spawns on **Tutorial Island** at `(6976, 6464)` with an empty
inventory. None of the bots work until it is out — no tools, no targets, and
`Man`/`Woman` do not exist there.

```sh
bun -e '
import { BotSDK } from "./sdk/index";
import { BotActions } from "./sdk/actions";
const sdk = new BotSDK({ botUsername: "mercbot01", password: "test",
  gatewayUrl: "ws://localhost:7780", connectionMode: "control", autoLaunchBrowser: false });
await sdk.connect();
await sdk.waitForCondition(s => s.inGame && !!s.player, 30000);
console.log(await new BotActions(sdk).skipTutorial());
process.exit(0);'
```

Verified output: `{"success":true,"message":"Tutorial skipped after 3 dialog clicks"}`,
landing at Lumbridge with the full kit — bronze axe, tinderbox, small fishing
net, shrimps, bucket, pot, bread, pickaxe, dagger, sword, shield, shortbow, 25
bronze arrows and the starting runes.

The skip only exists when `map_live = false`, i.e. a dev world. On production
there is no automation for the tutorial.

## Then run something

```sh
# The money maker. Start small and watch it.
bun bots/mercbot/thief.ts --target 200 --minutes 5

# Or drive it by hand from the dashboard console:
#   thieve            the pickpocket table at your level
#   where             position, hitpoints, what it looks like it is doing
#   npcs man          what is in range
#   control --force   take the character (disconnects the running script)
#   pickpocket man 5  five attempts, each outcome named
```

## What is verified, and what is not

**Verified against the live server just now:**

- Engine boots, gateway runs, headless client logs in and registers.
- The SDK connects in control mode and receives state.
- `skipTutorial()` works and grants the documented kit.
- A `Man` spawns beside the Lumbridge spawn point at `(3222, 3217)`.
- A pickpocket returns `You pick the man's pocket.` and a Thieving level-up.
- Twelve attempts paid **+9 gp** — three successes at exactly three coins,
  which is what `pickpocket.dbrow` says and what the Rust model predicts.

**Not yet verified end to end:**

- The **forage circuit** — fishing at the swamp, chopping, lighting, cooking.
  The `net` naming bug meant it could never have run before now; the fix is in
  but the loop itself has not completed a lap against a live world.
- **`thief.ts` as a whole run**, including recovery when hitpoints get low.
- The **shop flip** and anything touching the chain: the local server logs
  `[bridge] exchange closed (no registry)`, so the in-game exchange is inert
  here. The Rust side talks to mainnet and is unaffected.
- **Random events.** `NODE_RANDOM_EVENTS` defaults to false, so nothing has
  exercised that path.

## Three traps that cost me an hour

Worth knowing before you debug anything.

**`waitForReady(n)` is milliseconds, not ticks.** `waitForReady(6)` means "give
up after six milliseconds" and returns immediately. `waitForTicks(n)` is the one
that waits for game time. Every pacing loop in this bot was wrong until it was
run for real.

**The player's `x`/`z` are not world coordinates.** In Lumbridge the player
reports `x,z = 6976,6976` and `worldX,worldZ = 3222,3222`. Every NPC and loc
reports *world* coordinates in its own `x`/`z`. Mixing them gives distances
wrong by thousands of tiles.

**Item names are display names.** The tutorial's `net` is `Small fishing net` in
the inventory. Matching on the script id finds nothing.

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

On Windows `cmd.exe` the last line is:

```bat
xcopy /E /I /Y ..\solana_rs\gamebot bots\mercbot
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

`cmd.exe` has no inline env-var syntax — `BUILD_VERIFY=false bun ...` fails
there. Set it first, on its own line (a trailing space before `&&` ends up
*inside* the value, so do not chain it):

```bat
cd server\engine
bun install
set BUILD_VERIFY=false
bun run src/app.ts
```

PowerShell: `$env:BUILD_VERIFY = "false"` then `bun run src/app.ts`.

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

On `cmd.exe`, either `notepad bots\mercbot\bot.env` and paste, or:

```bat
(
echo BOT_USERNAME=mercbot01
echo PASSWORD=test
echo SERVER=localhost:8888
echo GATEWAY_URL=ws://localhost:7780
echo PROFANITY_FILTER=false
) > bots\mercbot\bot.env
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

The SDK's README points at a browser client on `/bot?bot=…&password=…`, and on
this build **it does not work**: the page loads, 404s on a resource, and never
registers with the gateway — no `[Gateway]` line ever appears for it. Use the
headless runner. See *Watching it play* below for how to actually see the
character.

### 4 — the dashboard

```sh
bun bots/mercbot/dashboard/server.ts --server localhost:8888
```

Open <http://localhost:8420>. It attaches as an **observer**, so it never
disturbs whatever is driving the bot.

## Four terminals means four terminals

Each of the first three blocks **stays running** and occupies its window. The
gateway does not return to a prompt; neither does the engine or the client. Open
a new terminal for each.

Two traps that follow from that on `cmd.exe`:

* **`set` does not carry between terminals.** `set BUILD_VERIFY=false` has to be
  run in the same window as `bun run src/app.ts`.
* **The `cd` lines above are written from the mercantile root**, not from
  wherever the previous block left you. Use an absolute path if in doubt:
  `cd F:\path\to\mercantile\server\engine`.

And do not paste a whole block at once into `cmd`: a comment line lands on the
end of the previous command rather than being ignored. One line at a time.

## A note on Windows

Everything here was verified on Linux. The four services are all `bun`, which is
cross-platform, and the paths inside the scripts are all relative — but two
things differ and both are called out above: `cp` → `xcopy`, and inline env vars
→ `set`. Forward slashes work fine as arguments to `bun` on Windows, so
`bun bots/mercbot/dashboard/server.ts` is correct as written.

If `bun` is not installed: <https://bun.sh/docs/installation> —
`powershell -c "irm bun.sh/install.ps1 | iex"`.

## The first thing to do: skip the tutorial

A fresh account spawns on **Tutorial Island** at `(6976, 6464)` with an empty
inventory. None of the bots work until it is out — no tools, no targets, and
`Man`/`Woman` do not exist there.

It ships as a script, so there is no shell quoting to get wrong. From the
mercantile root:

```sh
bun bots/mercbot/tools/skip-tutorial.ts
```

Verified output: `{"success":true,"message":"Tutorial skipped after 3 dialog clicks"}`,
landing at Lumbridge with the full kit — bronze axe, tinderbox, small fishing
net, shrimps, bucket, pot, bread, pickaxe, dagger, sword, shield, shortbow, 25
bronze arrows and the starting runes.

The skip only exists when `map_live = false`, i.e. a dev world. On production
there is no automation for the tutorial.

## Then run something

The money maker, kept short so you can watch it:

```sh
bun bots/mercbot/thief.ts --target 200 --minutes 5
```

Or drive it by hand from the dashboard console:

| command | what it does |
| --- | --- |
| `thieve` | the pickpocket table at your level |
| `where` | position, hitpoints, what it looks like it is doing |
| `npcs man` | what is in range |
| `control --force` | take the character (disconnects the running script) |
| `pickpocket man 5` | five attempts, each outcome named |

## Watching it play

The bot has no screen. The headless runner is a protocol connection, and the
dashboard shows numbers, not a game view. To watch the character move, log a
**second** character in as a spectator:

1. Open <http://localhost:8888/rs2.cgi>.
2. Create any second account — authentication is disabled locally, so any
   name and password work.
3. Walk it to Lumbridge, around `(3222, 3218)`.
4. Your bot is standing there. Start `thief.ts` and watch it work.

This costs nothing and disturbs nothing: it is just another player in the world.
Do **not** open `/bot?bot=<yourbot>` for this — besides not registering with the
gateway, a second client on the same account takes the session over, and the
engine logs `session takeover` as it drops the first one.

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

**Verified as broken:** the `/bot?bot=…&password=…` browser client the SDK
README suggests. It serves a page and then never registers with the gateway.

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

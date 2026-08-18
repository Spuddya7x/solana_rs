# Operating this bot — instructions for an agent

You are being asked to bring up a local RuneScape private server and run a bot
against it. This file is the operational contract: exact commands, the line that
proves each step worked, and the failure modes that have actually happened.

**Say `bots/mercbot/OPERATIONS.md` is being followed, then follow it.** Do not
improvise the order — the services depend on each other, and three of the four
never return to a prompt.

## What you are starting

```
  engine :8888   the game world itself
      ▲
      │ plays as the character
  lite client    a headless protocol client, logged in as the bot account
      ▲
      │ ws
  gateway :7780  routes actions/state between clients and scripts
      ▲   ▲
      │   └── dashboard :8420   observer — read-only GUI + console
      │
   thief.ts      controller — actually drives the character
```

Only **one** controller may exist at a time: the gateway is last-controller-wins
and silently disconnects the previous one. Observers are unlimited and never
pre-empt anything.

## Preconditions

Check these before starting anything; each has a one-line fix.

| Check | Command | If it fails |
| --- | --- | --- |
| `bun` present | `bun --version` | Install from <https://bun.sh> |
| repo layout | `dir bots\mercbot\thief.ts` (or `ls`) | `xcopy /E /I /Y ..\solana_rs\gamebot bots\mercbot` from the mercantile root |
| engine deps | `dir server\engine\node_modules` | `cd server/engine && bun install` |
| gateway deps | `dir server\gateway\node_modules` | `cd server/gateway && bun install` |
| credentials | `type bots\mercbot\bot.env` | create it — contents below |

`bots/mercbot/bot.env`:

```ini
BOT_USERNAME=mercbot01
PASSWORD=test
SERVER=localhost:8888
GATEWAY_URL=ws://localhost:7780
PROFANITY_FILTER=false
```

`GATEWAY_URL` is **required**. Without it the gateway address is derived from
`SERVER`, giving `ws://localhost:8888` — the engine's web port, not the gateway.
Everything appears to start and then silently never attaches.

All commands below are run **from the mercantile repo root** unless stated.

## Bring-up, in order

Each of steps 1–3 runs forever. Start each as a **background process** and wait
for its success line before starting the next. Do not chain them in one shell.

### 1. Engine

```sh
cd server/engine && BUILD_VERIFY=false bun run src/app.ts
```

Windows `cmd.exe` has no inline env vars — there, run `set BUILD_VERIFY=false`
as its own command first, in the same shell.

Wait for (first boot packs the cache, allow **3 minutes**):

```
World ready: Visit http://localhost:8888/rs2.cgi
```

`[bridge] exchange closed (no registry)` on the next line is expected and
harmless — the in-game exchange needs a registry file only the live world ships.

Without `BUILD_VERIFY=false` it exits with `.npc checksum mismatch!`, because
mercantile patches the content away from upstream's checksums.

### 2. Gateway

```sh
bun server/gateway/gateway.ts
```

Wait for:

```
[Gateway] Gateway running at http://localhost:7780
```

### 3. Game client

```sh
bun server/webclient/src/lite/runner.ts mercbot
```

Wait for **both** lines — the first alone means it reached the game but not the
gateway, and nothing will be able to drive it:

```
[lite-runner] 'mercbot01' logged into localhost:8888
[lite-runner] Gateway connected, registering as 'mercbot01'
```

The argument is the **bot directory name** (`mercbot`), not the account name.

### 4. Tutorial skip — once per account

```sh
bun bots/mercbot/tools/skip-tutorial.ts
```

A fresh account spawns on Tutorial Island with an empty inventory. Nothing works
until this runs: no tools, and no `Man` or `Woman` to pickpocket. Expect:

```
before: (3094,3106)
Tutorial skipped after 3 dialog clicks
after:  (3222,3222)
kit:    Bronze axe, Tinderbox, Small fishing net, Shrimps, ...
```

Already-done accounts print `already out of the tutorial` and exit 0. That is
success, not a failure to report.

This exits when finished — it is not a service.

## The three things you will be asked for

### "Start the server"

Steps 1–3 above, plus step 4 if the account has never run before. Report the
success line from each, and the character's position from the skip output.

### "Run the thieving script"

```sh
bun bots/mercbot/thief.ts --target 200 --minutes 5
```

This is a **controller**. Starting it disconnects any other controller, which
includes the dashboard if someone ran `control --force` in its console.

Keep the target and the minute cap small unless asked otherwise — an unattended
run that dies at 0 hitpoints drops its coins. Report the closing summary line,
which gives gp earned, attempts, success rate and gp/hour.

Options: `--target <gp>`, `--minutes <cap>`, `--recover forage|idle`,
`--trip <minutes of thieving one forage trip funds>`.

### "Start a browser version for me to monitor, with the bot control GUI"

Two separate things, both needed:

**The control GUI** — start as a background process:

```sh
bun bots/mercbot/dashboard/server.ts
```

Wait for `[dashboard] attached as observer`, then open
<http://localhost:8420>. Live skills with XP/hour and time-to-level, inventory,
activity log, and a command console. It attaches read-only, so it cannot disturb
a running script.

**The game view** — the bot has no screen of its own; the lite client is a
protocol connection with nothing to render. To see the character, open
<http://localhost:8888/rs2.cgi>, register any second account (authentication is
disabled locally, so any name and password work), and walk it to Lumbridge
around `(3222, 3218)`. The bot is standing there.

Tell the user they must create that spectator account themselves in the browser
— it needs a human at the keyboard, and it is a *different* account from the
bot's.

**Do not** open `http://localhost:8888/bot?bot=<name>&password=<pw>` for this.
It is in the SDK's README and it does not work on this build: it serves a page,
404s on a resource, and never registers with the gateway. It also takes the
session over from the lite client if the account matches.

## Failure modes that have actually happened

| Symptom | Cause | Fix |
| --- | --- | --- |
| `.npc checksum mismatch!` | `BUILD_VERIFY` not set in *that* shell | re-run `set BUILD_VERIFY=false` there |
| `File not found "src/app.ts::"` | a comment pasted onto the command | one command per line |
| Dashboard starts but stays "waiting" | gateway derived from `SERVER` | add `GATEWAY_URL` to `bot.env`, or pass `--gateway ws://localhost:7780` |
| `Module not found ".../skip-tutorial.ts"` | `bots/mercbot` copied before the file existed | `git pull` in `solana_rs`, re-copy |
| Engine logs `session takeover` | two clients on one account | run the lite client **or** a browser client, never both |
| Bot does nothing, no errors | still on Tutorial Island | run the skip (step 4) |
| Script runs but never paces | old copy without the tick fix | `git pull`, re-copy |

## Checking state without disturbing anything

```sh
curl -s http://localhost:8420/session          # JSON: position, hp, skills, coins
curl -s http://localhost:8420/command -d 'where'
curl -s http://localhost:8420/command -d 'thieve'
```

The console's command tiers: planning commands (`thieve`, `forage`, `shop`,
`spawns`, `safespots`) need no game at all; inspection (`skills`, `inv`, `where`,
`npcs`, `locs`, `ground`, `log`, `say`) needs the observer; `walk`, `pickpocket`
and `eat` need control and are refused until `control --force`, **which
disconnects the running script**. Never issue that while `thief.ts` is running
unless the user explicitly asked to take the character.

## Teardown

Stop in reverse order: scripts, dashboard, client, gateway, engine. The engine
writes player state on logout, so stopping the client before the engine is what
keeps the account's progress.

## Ground truth

Item names in the inventory are **display** names (`Small fishing net`, not
`net`). The player's position is `worldX`/`worldZ` — plain `x`/`z` is a
different space entirely and reads ~6976 in Lumbridge. `waitForReady(n)` takes
**milliseconds**; `waitForTicks(n)` is the one that waits for game time. All
three of these have caused silent bugs here.

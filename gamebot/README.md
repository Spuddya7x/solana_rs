# gamebot — the in-game half

These scripts play the RuneScape side of the loop: train Magic, cast alchemy on
items the on-chain side bought, and move value across the Exchange Clerk. They
use the [rs-sdk](https://github.com/MaxBittker/rs-sdk) BotSDK that ships inside
the [mercantile](https://github.com/MidTermDev/mercantile) repository, so they
run from a mercantile checkout rather than from here.

## Why this exists

`mercbot` (the Rust half) can buy an item near its pool floor — `0.9 × lowalch`,
which is `0.36 × cost`. Nothing on chain will pay more than that for it in size,
because a pool's bid is only the GP earlier buyers put in. The buyer who *will*
pay more is the game itself: **High Level Alchemy pays `0.6 × cost`**, from
nothing, to anyone with level 55 Magic and a nature rune.

That is the whole trade, and it is why the two halves belong in one repository.

```
mercbot alch-scan          -> which items clear the margin right now
mercbot run                -> buys them near the floor
Exchange Clerk (deposit)   -> tokens burned, items claimed in game
gamebot alch-loop.ts       -> 0.6 x cost in GP, one cast per 3 seconds
Exchange Clerk (withdraw)  -> GP minted back to the wallet
```

## Install

```sh
git clone https://github.com/MidTermDev/mercantile
cd mercantile && bun install
bun bots/create-bot.ts alchbot          # makes bots/alchbot/{script.ts,bot.env}
cp -r /path/to/solana_rs/gamebot/* bots/alchbot/
```

The scripts import `../../sdk/...`, matching the layout `create-bot.ts` produces,
and read credentials from the `bot.env` beside them. Run the world locally with
`bun server/start.ts` (or point `bot.env` at a hosted server).

## Scripts

| Script | What it does |
|---|---|
| `campaign.ts` | **The whole account.** Works out what stage it is at and does the next thing. |
| `moneymaker.ts` | Funds a fresh account from ground spawns. |
| `thief.ts` | Picks pockets in Lumbridge and fishes its own food to survive it. |
| `train-magic.ts` | Trains Magic to a target level, picking the best method available. |
| `alch-loop.ts` | Alchs a stack of items and optionally withdraws the GP to the wallet. |
| `lib/spells.ts` | Spell component ids, levels, runes, XP — derived from the game's config. |
| `lib/alch.ts` | Spell-on-item casting with XP-based completion detection. |
| `lib/bridge.ts` | Exchange Clerk dialogue: claim, withdraw GP, withdraw items. |
| `lib/economics.ts` | Alch values and break-evens, mirroring the Rust `alch` module. |
| `lib/supply.ts` | Buying runes from shops with no entry requirements. |
| `lib/thieving.ts` | Pickpocket table, the engine's success roll, and the hitpoint floor. |
| `lib/forage.ts` | Fish, chop, light, cook: making food where nobody sells any. |
| `lib/money/` | The ground-spawn circuit, shop pricing, and verified safespots. |
| `lib/nav/` | Navigation: gazetteer, router, executor. See below. |
| `tools/build-places.ts` | Regenerates the gazetteer from the game's map data. |
| `dashboard/server.ts` | Watches a running bot read-only and serves the session window. |
| `dashboard/stats.ts` | Turns observed world states into rates, ETAs and an activity log. |
| `dashboard/commands.ts` | The console: planning, inspection, and the gate on taking control. |
| `dashboard/preview.ts` | Serves the window against a synthetic session, no game needed. |
| `tsconfig.json` | Typechecks this bot. The repo's own config does not cover `bots/`. |

## Making the first 20,000 GP

Training to 55 costs about 113,000 GP of runes; a new character has 25. Three
things were measured before picking a route, and two of them killed the obvious
answers:

* **Cow hides are worth nothing here.** `cow_hide` has `cost = 1`, and shops pay
  `cost x multiplier / 1000` with multipliers of 600–700 — so a hide sells for
  **0 GP**. Raw beef, bones, feathers and raw chicken are all `cost = 1` too. The
  classic hide run does not work in this economy.
* **Low-level monster drops are worth single digits.** Reading each drop table's
  own `random(128)` thresholds, a barbarian averages ~11 GP a kill. Every table
  worth having (dragons, demons, giants) belongs to something a fresh account
  cannot fight.
* **Ground spawns pay properly.** Items on the floor respawn every 100 ticks — a
  minute — for no levels and no combat. Two clusters near Varrock hold nearly all
  the safe value:

| Cluster | Contents | Shop value |
|---|---|---|
| Varrock sewers (3194, 9822) | ruby ring, gold necklace, gold bar, gold ore | ~1,755 GP |
| Varrock surface (3254, 3452) | iron platebody, iron platelegs, iron sword | ~558 GP |

The surface cluster is the **splash kit** — iron platebody and platelegs are −30
and −21 magic attack — so the opening loop also equips the account for training.
Everything else worth 50 GP or more is in the wilderness, where an unattended bot
is someone else's loot.

```sh
bun bots/<name>/moneymaker.ts --target 20000
```

## Thieving: the bootstrap

The fastest money a brand-new account can make, and the only one that needs no
capital, no levels, no equipment and no walking. A `man3` stands three tiles from
where `tutorial_complete` teleports you, his pocket always holds exactly three
coins, and seven attempts in ten succeed at Thieving 1.

```sh
bun bots/<name>/thief.ts --target 5000
bun bots/<name>/thief.ts --recover idle --minutes 20
```

Two things are worth knowing before starting a long run.

**It is members-only.** `attempt_pick_pocket` opens with `if (map_members =
^false)`, and `map_members` is the **world** flag `Environment.node.members`, not
a per-zone one. Mercantile's default is `members: true`, so Lumbridge works; on a
free world none of this does.

**The healing is most of the program.** `~fail_pick_pocket` stuns for eight ticks
and takes a hitpoint, which at this cadence is **434 damage an hour** against
**60** of passive regeneration (`settimer(health_regen, 100)` — one hitpoint a
minute). Left alone the account dies in about four minutes, and `~damage_self`
queues `player_death` the moment hitpoints hit zero, dropping the coins the run
exists to collect. `lib/thieving.ts` holds a hard floor of two failures'
headroom; `--recover` picks how the bill gets paid.

### Recovering: `forage` or `idle`

Buying food is the obvious answer and it does not survive contact with the shop
table. Of the 64 counters a fresh account can reach, exactly one stocks food —
Wydin's Food Store in Port Sarim, 283 tiles away, selling **cabbage that heals
one**, with bread at base stock zero so there is nothing on the shelf.

So `forage` fishes. The circuit is fixed by the map and happens to be tidy:

```
  Lumbridge (3222,3218)  ── 115 tiles ──▶  swamp shrimp (3267,3148)
         ▲                                        │
         └──── 55 ──── tree (3253,3194) ◀── 60 ───┘
```

That tree sits *exactly* on the straight line home, so the chop, the fire and the
cook cost **zero detour** — which is why that tree and not one of the other 196 in
the area. `(3265,3215)` is the equally-free alternative. The tutorial grants the
bronze axe, tinderbox and small fishing net, so the whole circuit is free to run.

`idle` just stops and waits. At one hitpoint a minute, recovering one minute of
thieving takes seven minutes of standing still — it is there for accounts with no
tools and for short supervised runs, not as a real strategy.

### What it actually pays

`mercbot thieve-plan --thieving 1 --fishing 1 --cooking 1` costs the whole thing.
The ladder, in GP per hour of **wall clock** with the food loop priced in:

| | Thieving | Fishing | Cooking | uptime | net GP/h |
| --- | ---: | ---: | ---: | ---: | ---: |
| fresh account | 1 | 1 | 1 | 46% | 1,446 |
| farmers | 10 | 1 | 1 | 43% | 3,075 |
| + fishing | 10 | 20 | 1 | 56% | 4,012 |
| + cooking | 10 | 1 | 34 | 58% | 4,163 |
| both | 10 | 20 | 34 | 69% | 4,951 |

Two results worth acting on:

* **Thieving 10 is the single biggest upgrade** — a doubling, and it costs about
  seven minutes of picking men's pockets. Farmers pay nine coins for the same
  one-hitpoint stun, so they are worth half the hitpoints per 1,000 GP that men
  are. Everything else on the ladder is a third or less.
* **Cooking 34 stops the burning.** `cooking_generic_shrimp` rolls
  `successchance 128,512`, which is 129/256 at level 1 — half the catch is lost —
  and does not reach a guaranteed 256 until level 34. Below that every cooked
  shrimp costs two raw ones.

An inventory of 25 shrimp is 75 hitpoints, which is about ten minutes of farmers.
That short trip length, not the thieving, is what caps the rate.

## Watching a run

```sh
bun bots/<name>/dashboard/server.ts          # http://localhost:8420
bun bots/<name>/dashboard/preview.ts         # the same window, synthetic data
```

A separate window that watches a bot **without touching it**. The gateway's SDK
protocol has two connection modes and the second is what makes this work:

* `control` — full actions, and connecting **pre-empts any existing
  controller**. Two control connections fight; the newcomer wins.
* `observe` — read-only state and chat plus `say`. Never pre-empts, is never
  pre-empted, and observers coexist freely. `sdk_action` from an observer is
  rejected by the gateway unless it is `say`, so this is the protocol's rule
  rather than a convention here.

So the dashboard attaches as an observer, gets the same `BotWorldState` frames
the controller sees, and works against any script in this directory — including
one that was already running before the window opened. Closing it changes
nothing. The scripts report nothing and do not know it exists.

Everything shown is **derived** from those frames by `dashboard/stats.ts`, which
is where the correctness lives and is tested on its own. Two things that pass
for details and are not:

* **`baseLevel` is the true level, `level` is the boosted or drained one.** For
  a thieving bot this is the difference between a working display and a broken
  one: its Hitpoints are drained essentially all the time, so reading `level`
  reports the account as lower-levelled and aims the next-level target at a
  threshold it passed hours ago. The skills table shows the true level with the
  drained value beside it, the way the client does — `10 (7)`.
* **Coins are inventory, not a ledger.** Selling, dropping and dying all move
  the number and none of them are earnings, so it is labelled *carried*, not
  *profit*, and it is allowed to go negative. XP is monotonic and is the axis to
  believe when the two disagree.

Level-ups are logged off the level itself rather than the game's congratulation:
the message feed is a bounded rolling window, so a busy tick can push one out,
but a level cannot be missed. The activity line is read from the engine's own
`mes(...)` strings — `pick_pocket` says "You pick the man's pocket" and
`~fail_pick_pocket` says "You fail to pick the pocket", which is how the paint
tells a successful run from a stunned one.

### The console

The window carries a command line, and what it can do is decided by the same
mode split. Commands declare what they need and the dispatcher refuses anything
the connection is not entitled to — the gate sits in `execute`, not in the
handlers, so a new command cannot forget it.

| Tier | Needs | Commands |
| --- | --- | --- |
| **Planning** | nothing at all | `thieve`, `forage`, `shop`, `spawns`, `safespots`, `help` |
| **Live** | an observer | `skills`, `inv`, `where`, `npcs`, `locs`, `ground`, `log`, `say` |
| **Control** | the character | `control`, `release`, `walk`, `pickpocket`, `eat` |

Planning commands are the numbers `mercbot` prints, without leaving the window
or having a game attached: `thieve 12` gives the pickpocket table at that level,
`forage 12` sizes the trip, `shop lobster 150` finds the counter. `shop` says
which kind it found, because a general store buys *anything* at 400/1000 and
reporting that as a find would be misleading — it is low-alchemy value against a
36% floor and gone by the third unit.

`say` is the one action an observer may send, so the console can talk through
the bot without taking it.

**`control` is the sharp one.** The gateway is last-controller-wins: connecting
in control mode disconnects whatever is driving the bot, and it does not come
back on its own. So `control` refuses on the first attempt and explains; only
`control --force` escalates. `release` drops back to observing and says plainly
that the evicted script is still gone and needs restarting.

### What a pickpocket actually grants

`pick_pocket_check_for_reward` is **not** a pick-one table, and reading it as one
gets the numbers wrong. It walks the `loot` entries **backwards** with a
denominator that starts at 128 and shrinks by each numerator as it goes:

```
$roll = random($denominator)          // the denominator *before* the subtraction
$denominator = $denominator - $numerator
if ($roll >= $denominator) { inv_add(...) }   // no return — the loop continues
```

Three consequences, all of which cost me a correction:

* **One success can grant several items.** There is no `return` after a hit,
  unlike the stall table's `stealing_check_for_reward`. A rogue can pay coins
  *and* air runes *and* wine in a single pick.
* **The dbrow's first entry is usually guaranteed**, because the numerators sum
  to 128 and drive the denominator to zero. The rogue's coins have numerator
  108, which reads like 84% and is actually **certain** — I had priced the rogue
  16% low.
* **Unless the weights sum to less than 128.** The farmer's single entry is
  **123**, so 5 successful picks in every 128 pay *nothing*. That is 8.65 coins
  a pick, not 9.

| target | level | per success | extras |
| --- | ---: | --- | --- |
| man/woman | 1 | 3 gp, always | — |
| farmer | 10 | 9 gp, 96.1% of the time | — |
| warrior | 25 | 18 gp, always | — |
| rogue | 32 | 25–40 gp, always | air runes 6.9%, wine 4.9%, lockpick 3.9%, poisoned iron dagger 0.8% |
| guard | 40 | 30 gp, always | — |
| fremennik | 45 | 40 gp, always | — |
| knight | 55 | 50 gp, always | — |
| watchman | 65 | 60 gp, always | bread, always |
| paladin | 70 | 80 gp, always | 2 chaos runes, always |
| gnome | 75 | 300 gp, **35.3%** | king worm always, toad 24.8%, gold ore 6.6%, earth rune 4.0%, fire orb 1.6% |
| hero | 80 | 200–300 gp, always | 2 death runes 7.1%, wine 5.0%, blood rune 4.0%, fire orb 1.6%, diamond 0.8%, gold ore 0.8% |

The gnome is the odd one: the guaranteed entry is a king worm, and the 300 gp is
the coin flip. `mercbot thieve-plan` counts coins only, because the runes and
worms have to be carried home and sold before they are income.

### Every way an attempt can end

`lib/thieving.ts` enumerates twelve outcomes, in the order the script tests
them. A bot that only checks "did coins go up" reads six different refusals as
the same silent nothing and hammers the NPC forever.

| outcome | trigger | costs hp? | retry? |
| --- | --- | --- | --- |
| `members-only` | `map_members = false` | no | never |
| `unknown-target` | no dbrow for the npc | no | never |
| `level-too-low` | `stat(thieving) < level` | no | not until levelled |
| `quest-locked` | `%viking < ^viking_complete` | no | never |
| `in-combat` | `%lastcombat + 8 > map_clock` | no | after 8 ticks |
| `stunned` | `%stunned > map_clock` | no | after the stun |
| `too-soon` | `%action_delay > map_clock` | no | **it re-queues itself** |
| `target-dead` | `npc_stat(hitpoints) = 0` | no | pick another |
| `random-event` | `afk_event = true` | no | deal with the event |
| `failed` | the roll missed | **yes** | after the stun |
| `success` | the roll hit | no | immediately |
| `died` | the stun took the last hitpoint | fatal | no |

`too-soon` looks like a bug and is not: the script calls `p_opnpc(3)` and
returns, so the engine retries by itself. Re-sending on it doubles the send rate
for nothing. `classify()` distinguishes all of them from the game messages, with
hitpoints as the tie-break — and deliberately returns `unknown` rather than
`success` for silence, since counting silence as a pick would inflate every rate
on the dashboard.

**Random events default to off** (`NODE_RANDOM_EVENTS=false`), but where they are
on they matter: `macro_event_general_spawn` can roll the maze and cube events,
which *teleport the character away*, and `%macro_event > 0` then blocks every
future event until it is resolved. `pickpocket` refuses to start next to a known
event NPC rather than walking into that unattended.

### The three actions

`walk`, `pickpocket` and `eat` are control-tier, so they need `control --force`
first. Each reports what actually happened rather than what was asked for:

* `walk <x> <z>` — `walkTo` succeeds on arriving *near enough*, so this prints
  where it stopped and how far short it is, rather than claiming arrival.
* `pickpocket [pattern] [count]` — names which of the twelve outcomes each
  attempt was, stops on a fatal one instead of hammering, and caps at 25 so one
  line cannot run away.
* `eat [food]` — healing clips at max, so a 3 hp shrimp eaten at 9/10 gives 1.
  This prints the hitpoints that landed, not the food's listing, and refuses at
  full health rather than wasting it.

The same dispatcher is on an HTTP endpoint, so a shell can drive it too:

```sh
curl -s localhost:8420/command -d 'thieve 12'
```

The server binds to loopback on purpose: the page has no authentication and the
observer socket behind it is already authenticated as the bot.

## Selling to shops

`lib/money/pricing.ts` mirrors the Rust `shops` module, and both port
`shop.rs2` exactly. The two facts that matter:

* Shops pay `cost x multiplier / 1000`. A **specialist** counter — one that
  deals in the item — uses 600–950, against a pool floor of 36% of cost, so an
  item bought on chain and sold there returns the alchemy margin with no Magic
  level at all. Every reachable **general store** uses 400, which is 40% of cost
  and only 11% over the floor. `mercbot shop-flip` ranks the former; the latter
  is for dumping loot.
* The price falls as you sell and rises when the shop is depleted, because
  `diff = current + sold - base`. Planning assumes base stock, which is neutral.
  At `haggle = 30` a general store's 11% is gone inside three units.

### Getting there

Only **64 of the 117 shops** are reachable by a fresh account walking from
Lumbridge. `tools/build-money.ts` establishes this with the game's own
pathfinder and stamps every shop with `accessible`, `barrier` and `walkTiles`;
`pickShop` drops the rest before it ranks anything.

| Barrier | Shops |
| --- | ---: |
| `unreachable` (water, off-graph) | 30 |
| `upstairs` (level > 0) | 14 |
| `quest: viking` | 6 |
| `gated: fishing 68` / `quest: dragonquest` / `quest: mcannon` | 3 |

Two gate mechanisms exist and both need checking. Most gates are **doors** —
`[oploc1,…]` handlers that test a quest or a stat before opening. The Fremennik
shops have no such door: the shopkeeper's own `[opnpc…]` script checks
`%viking < ^viking_complete`, so a door-only scan sends the bot a thousand tiles
to Rellekka to meet a merchant who will not trade with it.

`pickShop` ranks by **value per hour**, not value: the Ardougne fur stall pays
95% of cost but is 861 tiles out, and a ten-minute round trip only wins if it
pays more than five times as much as a two-minute one.

## Safespots

A safespot is a tile where the monster can be **seen** but cannot **reach** you,
and both halves were tested against the shipped collision data with the engine's
own routines:

* `hasLineOfSight(player -> npc)` must hold — the engine's `inApproachDistance`
  uses exactly this for spell and arrow range.
* `findLongPath(npc -> player)` must fail to arrive. Line-of-*walk* alone is not
  enough: it tests a straight line, and monsters walk around pillars. The weaker
  test marked every dungeon spawn safe; the real one does not.

**Yes, you can cast over the Lumbridge cow fence.** From a cow at (3254, 3258),
line-of-walk fails two tiles west but line-of-sight holds out to eleven — the
textbook safespot signature. Cows are a *safe training spot*, not an income:
their drops are worth nothing.

`lib/money/safespots.ts` carries the verified entries worth using once an account
can actually fight, richest first: black demons (706 GP/kill), fire giants (629),
jogres (587), greater demons (502), ice giants (181), and moss giants (129) —
the only one on the surface.

## The campaign

`campaign.ts` runs an account end to end:

```
equip     wear a splash kit so stat-reduction spells stay castable
stock     buy runes at Aubury or Betty — no entry requirements, 1,000+ deep
train     splash up the ladder to the level alchemy needs
produce   high alch bridged-in items into GP
cash out  send the GP to the wallet, where mercbot buys scarcity with it
```

The stage is **derived from observation, never remembered**. No saved state means
nothing to go stale: kill it mid-run, restart it, point it at an account someone
played by hand, or run it across a fleet at different stages — same command line,
correct behaviour.

```sh
bun bots/<name>/campaign.ts --target 55 --alch "rune platebody" --cycles 20
```

Nature runes are the one input it cannot buy: nothing sells them below 66 Magic,
so `produce` claims them from the chain side via the Exchange Clerk. Everything
else it sources itself.

## The training route, and why it is nearly free

Run `mercbot magic-plan --to 55` for the current numbers. Two routes, and the
choice between them is really a choice about rune supply:

**Route A — splashing, self-sufficient.** Every rune comes from Aubury or Betty,
who have no entry requirements and restock. Nothing else is needed.

| Range | Spell | XP/cast | Casts | Time | Runes |
|---|---|---|---|---|---|
| 1 → 3 | Wind Strike | 5.5 | 32 | 2 min | 224 GP |
| 3 → 11 | Splash Confuse | 13 | 91 | 5 min | 2,093 GP |
| 11 → 19 | Splash Weaken | 21 | 125 | 6 min | 2,875 GP |
| 19 → 39 | Splash Curse | 29 | 1,023 | 51 min | 23,529 GP |
| 39 → 55 | Splash **Crumble Undead** | 49 | 2,714 | 2.3 h | 84,134 GP |
| | | | **3,985** | **3.3 h** | **113,000 GP** |

Crumble Undead is the standout: 49 XP a cast on shop-bought chaos, air and earth
runes — the best XP *per GP* in the game below 66, and the best XP per hour that
needs no nature runes. It only affects skeletons, zombies, ghosts and shades, so
it wants the Varrock sewers rather than a chicken.

**Route B — alchemy, faster and cheaper but supply-gated.**

| Range | Spell | XP/cast | Casts | Time |
|---|---|---|---|---|
| 1 → 21 | as above | | 284 | 14 min |
| 21 → 55 | **Low Level Alchemy** | 31 | 5,214 | 2.6 h |
| | | | **5,498** | **2.8 h** |

Half an hour quicker, and the rune bill is ~51,000 GP instead of ~113,000 —
before counting the `0.4 × cost` each alch pays *back*, which makes it outright
profitable on items above ~220 GP of shop cost. The catch is in the casts
column: **5,214 nature runes and 5,214 items**. On-chain rune pools hold 100
units, so this route is gated on a rune supply that does not exist below 66
Magic unless you runecraft (level 44, and the world runs members content).

Route A is what a fresh account actually does. Route B is what you switch to
once the Wizards' Guild is open.

| 55 → 66 | **High Level Alchemy** | 65 XP | Every cast already profitable, and 66 opens the guild — see *Runes*. |

### Splashing, and why it is the fast route

Stat-reduction spells are worth three to five times a strike spell at the same
level, and they are only repeatable if you keep **missing**. Three things in the
engine make that work, each checked in the server scripts rather than assumed:

1. **XP is paid before the hit roll.** `pvm_default_spell` calls `~pvm_spell_cast`
   — runes deleted, `~give_spell_xp` paid — and *then* rolls for the hit. A
   splash keeps the full base XP.
2. **A splash never applies the debuff.** `~pvm_stat_change_effect` runs only on
   the success branch, and `~pvm_debuff_allowed` refuses to cast on an NPC whose
   stat is already lowered ("Your foe's attack has already been weakened"). So a
   *landed* Curse is what stops you training. Miss forever and one chicken lasts
   the whole grind.
3. **A magic attack bonus of −64 or worse guarantees the miss.** The roll is
   `effective_magic × (bonus + 64)`, and a hit needs
   `randominc(attack) > randominc(defence)`. At −64 the attack roll is zero or
   negative and can never win.

The kit, and the nice part — the magic penalty does not scale with tier, so
bronze is as good as rune at a four-hundredth of the price:

| Piece | Magic attack | Shop value |
|---|---|---|
| Bronze platebody | −30 | 160 |
| Bronze platelegs | −21 | 80 |
| Bronze kiteshield | −8 | 68 |
| Bronze full helm | −6 | 44 |
| Bronze warhammer | −4 | 47 |
| **Total** | **−69** | **399** |

`train-magic.ts` equips whatever it finds in the inventory, refuses to cast a
stat-reduction spell without the bonus (pass `--allow-hits` to override), and
warns if a cast ever lands.

Splashing needs **no nature runes** — Confuse, Weaken and Curse take only body,
water and earth, and Crumble Undead takes chaos, air and earth. Nature runes are
purely an *alchemy* cost.

161,618 XP separates 21 from 55: about 5,200 low alchs, roughly two and a half
hours of casting. Feed it with `mercbot`:

```sh
mercbot alch-scan --limit 20                       # pick a target
mercbot run --once                                 # buy near the floor
# bridge in through the Exchange Clerk, then:
bun bots/alchbot/train-magic.ts --target 39 --npc chicken
bun bots/alchbot/train-magic.ts --target 55 --undead skeleton
bun bots/alchbot/alch-loop.ts --alch "rune platebody" --claim --withdraw-gp
```

## Navigation

The SDK already solves tile-level movement — `bot.walkTo` pathfinds over the full
collision map (1M tiles, 2,603 doors), opens doors, and detects being stuck. What
it cannot do is anything that is not a walk, or answer "where is the nearest
bank". `lib/nav/` is that layer:

```ts
import { travelTo, NAV, capabilities, here } from './lib/nav';

await travelTo(bot, sdk, 'aubury');                  // named destination
const bank = NAV.nearestTagged(here(sdk), 'bank', capabilities(sdk));
```

* **Gazetteer** — 200+ places. Region names come from `maps/labels.txt`; banks,
  undead sites and chicken farms are clustered from the `==== NPC ====` sections
  of the map squares; shops, the sewer manhole and the Wizards' Guild are curated
  and individually sourced. Nothing is remembered — `bun tools/build-places.ts`
  regenerates it.
* **Router** — Dijkstra over *transitions*, asking the pathfinder for walk costs
  rather than storing a road network. Requirements are edges' preconditions, so a
  teleport the bot has no law runes for simply is not an edge, and the route
  found is one the bot can actually take.
* **Executor** — walks each step and **verifies arrival** before the next one, so
  a failed climb stops the run instead of leaving the bot casting at the wrong
  floor.

Transitions currently modelled: the five free teleports (level, runes and landing
coordinates from `magic_spells.dbrow`), the Varrock sewer manhole, and the
Wizards' Guild door and staircase. Adding more is a data edit in `lib/nav/links.ts`.

`bun test lib/nav/` runs the router tests — pure logic, no game needed.

## Runes

One nature rune per cast, and five fire runes unless a **staff of fire** is
equipped — buy the staff once and the fire runes stop mattering.

Nature runes are the recurring cost, and the supply routes are worth knowing:

* **On chain.** Nature runes are a tokenised item like any other, floor `7.2 GP`
  — but every pool is seeded with just **100 units**, runes included, so this is
  a trickle, not a supply. Live right now: 20 runes cost 11.3 GP each (29% price
  impact) and 60 cost 25.8 each (197%). Good for bootstrapping ~20 casts at a
  time; useless for running a business.
* **Wizards' Guild (Yanille).** 1,000 in stock, restocking, and the *only*
  unbounded source — but the door checks for **66 Magic**. That is 330,094 XP
  past level 55, about 5,100 high alchs, and every one of them already pays. It
  is the real target.
* **Runecrafting.** Level 44 Runecrafting at the Nature Altar, unbounded and
  free once you are there. The world runs members content by default, so this is
  open — it is simply a second grind.

The elemental runes for the training grind have no such problem: **Aubury**
(Varrock) and **Betty** (Port Sarim) both stock air, water, earth, mind and body
runes, 1,000–2,000 deep and restocking, with no requirements at all.

## Honest limits

* **These scripts have not been run against a live world.** They typecheck
  against the real SDK and the spell ids are derived from the game's own config
  and cross-checked against the SDK's published table, but the first run deserves
  a throwaway account and a close eye.
* **Typecheck with this directory's own config**, not the repo's: `bunx tsc
  --noEmit -p bots/<name>/tsconfig.json`. The root `tsconfig.json` includes only
  `*.ts`, `server/gateway/**` and `sdk/**`, so running it against the repo checks
  none of this and reports success either way. Adding the local config
  immediately turned up three SDK methods called by names that do not exist
  (`interactObject`, `useItemOnObject`, `useItemOnItem` — the real ones are
  `interactLoc`, `useItemOnLoc`, and the `chopTree`/`burnLogs` helpers).
* **Thieving rates assume a bot that never misclicks.** The cadence model gives a
  success two ticks and a failure the full eight-tick stun; a real run loses some
  of that to walking between spawns and to NPCs wandering out of range.
* **Wallet linking is manual.** The clerk shows a one-time code that has to be
  signed off-line with `bun chain/cli/link-wallet.ts <code>`. `lib/bridge.ts` can
  surface the code; it cannot sign for you.
* **This mints GP.** Alchemy creates coins from nothing, and withdrawing them
  mints GP tokens (or drains the bridge's vault). The profit is real but it is
  GP-denominated and self-diluting — see the root README's note on holding items
  rather than GP.
* **The developers allow botting here; that is what makes this legitimate.**
  These scripts are written for that world and no other.

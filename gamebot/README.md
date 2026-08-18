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
| `train-magic.ts` | Trains Magic to a target level, picking the best method available. |
| `alch-loop.ts` | Alchs a stack of items and optionally withdraws the GP to the wallet. |
| `lib/spells.ts` | Spell component ids, levels, runes, XP — derived from the game's config. |
| `lib/alch.ts` | Spell-on-item casting with XP-based completion detection. |
| `lib/bridge.ts` | Exchange Clerk dialogue: claim, withdraw GP, withdraw items. |
| `lib/economics.ts` | Alch values and break-evens, mirroring the Rust `alch` module. |
| `lib/supply.ts` | Buying runes from shops with no entry requirements. |
| `lib/money/` | The ground-spawn circuit, shop pricing, and verified safespots. |
| `lib/nav/` | Navigation: gazetteer, router, executor. See below. |
| `tools/build-places.ts` | Regenerates the gazetteer from the game's map data. |

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

## Selling to shops

`lib/money/pricing.ts` mirrors the Rust `shops` module, and both port
`shop.rs2` exactly. The two facts that matter:

* Shops pay `cost x multiplier / 1000`, multipliers being 600–950, against a
  pool floor of 36% of cost — so an item bought on chain and sold at a counter
  returns the alchemy margin with no Magic level at all. `mercbot shop-flip`
  ranks that; `moneymaker.ts` walks it.
* The price falls as you sell and rises when the shop is depleted, because
  `diff = current + sold - base`. Planning assumes base stock, which is neutral.

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
* **Wallet linking is manual.** The clerk shows a one-time code that has to be
  signed off-line with `bun chain/cli/link-wallet.ts <code>`. `lib/bridge.ts` can
  surface the code; it cannot sign for you.
* **This mints GP.** Alchemy creates coins from nothing, and withdrawing them
  mints GP tokens (or drains the bridge's vault). The profit is real but it is
  GP-denominated and self-diluting — see the root README's note on holding items
  rather than GP.
* **The developers allow botting here; that is what makes this legitimate.**
  These scripts are written for that world and no other.

# solana_rs — a framework trading bot for Mercantile

[Mercantile](https://github.com/MidTermDev/mercantile) tokenises every tradeable
item of a 2004-era RuneScape world on Solana. Each item is an SPL mint paired
against **GP** — the tokenised in-game currency — in a Meteora DAMM v2 pool,
seeded single-sided at `0.9 × lowalch` over the range `[P0, ∞)`.

This repository is a bot that trades that economy, in two halves that meet at the
game's Exchange Clerk:

| | |
|---|---|
| **`mercbot`** (Rust) | Reads the pools, runs strategies, executes swaps on paper or on chain. |
| **`gamebot/`** (TypeScript) | Plays the game side: trains Magic, casts alchemy, bridges value back. |

## The full loop

One account, start to finish, and each stage feeds the next:

```
gamebot/campaign.ts     fund -> equip -> stock runes -> train -> alch -> cash out GP
                                       |                            |
                        buys runes from Aubury/Betty      Exchange Clerk -> wallet
                                                                      |
mercbot run                                       accumulate: GP -> capped-supply items
```

`campaign.ts` derives its stage from what it observes rather than from saved
state, so it is safe to kill, restart, point at a half-trained account, or run
across a fleet at different stages with one command line. It funds itself from
respawning ground spawns — cow hides and low-level drops were measured and are
worth almost nothing in this world; see
[`gamebot/README.md`](gamebot/README.md#making-the-first-20000-gp).

```sh
bun bots/<name>/campaign.ts --target 55 --alch "rune platebody" --cycles 20
mercbot run                      # accumulate turns the GP into scarcity
```

## The trade

A pool's floor is `0.9 × lowalch = 0.36 × cost`. **High Level Alchemy pays
`0.6 × cost`.** An item bought at its floor and alched returns about **1.67×**
before costs — and unlike every other edge in this market, the counterparty is
the game itself, so it does not depend on anyone else showing up to trade.

```
mercbot alch-scan      which items clear the margin right now, from live quotes
mercbot run            buys them near the floor, within your risk limits
Exchange Clerk         burn tokens on chain, claim the items in game
gamebot alch-loop.ts   0.6 x cost in GP, one cast per three seconds
Exchange Clerk         withdraw the GP, minting it back to the wallet
```

Live mainnet, at the time of writing: every one of the 120 richest markets is
profitable to alch at about **28% margin** after price impact, the 1% pool fee
and nature runes.

## Quick start

```sh
cargo build --release
cp mercbot.example.toml mercbot.toml     # edit; paper mode is the default
./target/release/mercbot registry-sync   # fetch the item registry

./target/release/mercbot markets --limit 20 --sort premium
./target/release/mercbot alch-scan --limit 20
./target/release/mercbot quote rune_platebody buy 5
./target/release/mercbot run --once      # one paper tick against live pools
./target/release/mercbot run             # paper, until Ctrl-C
```

Nothing above sends a transaction. Live trading needs three independent gates:
`bot.mode = "live"`, `bot.allow_live = true`, and `mercbot run --live`.

## Layout

| Crate | What |
|---|---|
| `crates/mercantile-core` | Registry parsing, amount and price conversions, alchemy economics. |
| `crates/mercantile-dex` | `cp_amm` pool decoding, the program's swap math, swap building, RPC. |
| `crates/mercantile-bot` | The framework and the `mercbot` binary. |
| `gamebot/` | The in-game half: training, alchemy, bridging, navigation. See [`gamebot/README.md`](gamebot/README.md). |

## The framework

A strategy implements one method. Everything else is already in place.

```rust
pub trait Strategy: Send {
    fn name(&self) -> &str;
    fn on_market(&mut self, view: &MarketView<'_>) -> Vec<Signal>;
    fn on_fill(&mut self, fill: &Fill) {}
}
```

A `MarketView` carries the pool, its price history, the current position, and
live quoting (`view.quote_buy(5)`, `view.max_tradeable(...)`, `view.exit_depth_gp()`),
so a strategy reasons in real numbers rather than in indicators.

Signals are *intent*. They pass through the **risk manager**, which is the only
component that can authorise a trade — it sizes down to fit budgets, enforces
cooldowns, exposure and impact caps, and latches a kill switch on losses. Adding a
strategy therefore cannot widen the bot's risk. What survives goes to an
**executor**: `PaperExecutor` prices fills from the same live quotes and sends
nothing; `LiveExecutor` builds, signs and confirms the swap. Everything either one
does lands in an append-only JSONL journal that `mercbot report` summarises.

### Built-in strategies

| Strategy | Idea |
|---|---|
| `accumulate` | **The exit.** Converts GP into items whose supply cannot grow. Never sells. |
| `shop-arb` | The alchemy margin without the level 55 gate — the exit is an NPC counter. |
| `alch-arb` | Buys only stacks High Level Alchemy would pay for. Exits through the game, not the pool. |
| `alch-floor` | Buys near the permanent floor, sells into strength. |
| `mean-reversion` | Fades moves away from a rolling mean, gated on the floor premium. |
| `grid` | A ladder anchored to the floor — the one price here that never drifts. |

`mercbot strategies` describes them; `mercbot.example.toml` configures them.

## Two exits from a pool position

Alchemy is not the only buyer. A pool's floor is **36% of an item's cost**; a
**specialist** shopkeeper's `shop_buy_multiplier` is **600–950**, so selling at
the right counter returns 60–95%. That is the same 1.67× high alchemy pays at
the low end, better at the high end — and it needs **no Magic level, no runes,
and no level 55 gate**.

The word "specialist" is doing the work. Every **general store** a fresh account
can reach pays 400 — 40% of cost, which is exactly low-alchemy value and only
11% over the pool floor. With `haggle = 30` that 11% is gone by the third unit,
so a general store is somewhere to dump loot, not somewhere to flip.

```
$ mercbot shop-flip --scan 220 --limit 6 --bankroll 50000
market                 items      buy GP     profit margin  tiles     GP/hour  sell to
adamant_platelegs         10       30788       7932    26%    108      403324  Louies' Armoured Legs Bazaar.
adamant_plateskirt        10       30788       7932    26%    113      386929  Ranael's Super Skirt Store.
mithril_battleaxe          5        3927        803    20%     27      150557  Bob's Brilliant Axes.
mithril_platelegs         10       12829       2901    23%    108      147502  Louies' Armoured Legs Bazaar.
mithril_plateskirt        10       12829       2901    23%    113      141506  Ranael's Super Skirt Store.
diamond_necklace           9       16121       4383    27%    254       99991  Grum's Gold Exchange.
```

Both decays are modelled rather than assumed: the pool charges more per item as
you buy, and the shop pays less per item as you sell (`haggle/1000` of cost per
unit), so `items` is where the two curves cross. Revenue is priced with the
counter at its **base stock** — a depleted shop pays far more, up to 5.7× cost,
but planning on that promises profit that only exists if nobody traded there
recently.

### Can the bot get there, and is the trip worth it?

Two filters that a naive scanner skips, and both change the answer.

**Reachability.** Only **64 of the 117 shops** can be walked to from Lumbridge by
a fresh account. `tools/build-money.ts` decides this by asking the game's own
pathfinder, and records why not:

| Barrier | Shops | What it is |
| --- | ---: | --- |
| `unreachable` | 30 | Across water or otherwise off the walkable graph |
| `upstairs` | 14 | On a level above 0, reached by a staircase |
| `quest: viking` | 6 | The Fremennik counters, behind The Fremennik Trials |
| `gated: fishing 68` | 1 | The Fishing Guild |
| `quest: dragonquest` | 1 | Oziach, behind Dragon Slayer |
| `quest: mcannon` | 1 | Behind Dwarf Cannon |

Gates come in two shapes and it takes both detectors to find them all. Most are
**doors**: an `[oploc1,…]` handler that checks a quest or a stat before it opens.
The Fremennik shops are not — they check `%viking < ^viking_complete` inside the
**shopkeeper's own** `[opnpc…]` script, with no door involved, so a door-only
scan walks you all the way to Rellekka and finds a merchant who will not trade.

**Trip time.** The best-paying counters are the furthest: the Ardougne fur stall
pays 95% of cost and sits 861 tiles from Lumbridge, about ten minutes of round
trip. Flips are therefore ranked by **GP per hour**, not profit per trip, and a
tie on price goes to the nearer counter — `uncut_diamond` fetches 70% at both
Herquin's and the Gem Trader, and the Gem Trader is 68 tiles away rather than
357.

**Capital.** GP/hour says nothing about the GP tied up, so without `--bankroll`
the top of the board is whatever item is most expensive: a dragon square shield
is an 8% general-store flip, but 8% of 184,000 GP is not a trade an account with
20,000 can take. `--bankroll` caps each size to what is actually affordable, and
`--min-margin` (default 0.1) hides the razor-thin rows.

Wilderness counters are excluded. The wilderness is bounded in **x as well as z**
(`wilderness_zones.dbrow`: x 2944–3391, z 3520–6399), so a z-only test condemns
Rellekka and the north-west; the real bounds put only two shops inside it.

## Where the first coins come from

Neither exit above works on an account with no GP, so something has to go first.
`mercbot thieve-plan` costs the cheapest option there is — picking pockets in
Lumbridge, which needs no capital, no levels and no equipment:

```
$ mercbot thieve-plan --thieving 1 --fishing 1 --cooking 1
target       thv    food fish cook  tiles  gross/h  uptime  net GP/h
farmer        10  shrimp    1    1    115     7209     43%      3075
man/woman      1   trout   20   34     50     3142     80%      2498
man/woman      1  shrimp    1    1    115     3142     46%      1446
```

A man's pocket always holds exactly three coins — `pick_pocket_check_for_reward`
rolls `random(128)` against a denominator his single 128-weight entry drives
straight to zero — and seven attempts in ten succeed at Thieving 1.

The interesting column is `uptime`. A failure stuns for eight ticks and takes a
hitpoint, which works out at **434 damage an hour** against **60** of passive
regeneration, and there is no food shop worth walking to: of the 64 reachable
counters exactly one stocks food, and it sells cabbage that heals one. So the
account has to fish its own, and at base levels that costs more than half the
clock. `gamebot/thief.ts` runs the loop; `gamebot/README.md` has the circuit.

The single biggest upgrade is **Thieving 10**, which costs about seven minutes
and doubles the rate by unlocking farmers — nine coins for the same one-hitpoint
stun. Cooking 34 is next: below it, `successchance 128,512` burns half the catch.

## Getting an account to level 55

The bot that buys is useless without an account that can alch. That is
`gamebot/`'s job, and the route is short:

`mercbot magic-plan` computes the route; `gamebot/train-magic.ts` walks it.

```
$ mercbot magic-plan --to 55
cheapest route, magic 1 to 55:
  spell             levels     casts   hours    rune GP  cast on
  wind strike        1-11        247     0.2       1729  any npc
  weaken            11-19        125     0.1       2875   splash
  curse             19-39       1023     0.9      23529   splash
  crumble undead    39-55       2714     2.3      84134   undead
  total: 4109 casts, 3.4 hours, 112267 GP of runes
```

**3.3 hours and ~113,000 GP of runes, entirely from open shops** — Aubury and
Betty stock every rune this route needs, 1,000+ deep and restocking, with no
requirements. Splashing needs no nature runes at all.

Two things make it work, both read out of the engine rather than assumed. Spell
XP is paid *before* the hit roll, so a splash trains at full rate. And a *landed*
stat-reduction spell debuffs the target, which blocks the next cast — so missing
is what sustains the grind. A −64 magic attack bonus guarantees the miss; a full
bronze kit is −69, for 399 GP, because the penalty does not scale with tier.

Allowing alchemy (`--with-alchemy`) is faster and cheaper still — 2.8 hours,
~51,000 GP, and profitable on items above ~220 GP of shop cost — but it needs
5,214 nature runes and 5,214 items. On-chain rune pools hold 100 units, so that
route only opens at **66 Magic**, the Wizards' Guild door and the game's only
unbounded nature rune supply. 66 is the real target; 55 is where the paying
starts.

## Can real money leave?

**Not today, but the developer has said it will.** Checked rather than assumed:
DexScreener lists 30 pairs for the GP mint and **every one is item/GP** — zero
pairs price GP against anything, zero have a USD price, zero have USD liquidity.
Jupiter returns `NO_ROUTES_FOUND` for GP to SOL. The economy is closed as it
stands.

What has been said publicly (MidTermDev, 18 Aug 2026) changes the outlook rather
than the present state:

* GP is the **only** token that will be issued; there is no separate project
  token. In-game gold becomes on-chain GP once players can log in.
* **A SOL pair will be created by the admin wallet** when the game goes live,
  deliberately rather than permissionlessly, to pre-empt malicious pools.
* Trading it will carry a **2% development tax** against roughly 65 SOL of setup
  cost for the mint and its ~1,400 Meteora pairs.

That last point is worth checking rather than fearing, because a transfer tax on
the mint would land inside every quote this bot makes. It does not: the mint
`123B7bdJzDYGkrAg7i3JUi5TaHYP47dqmSiR5qPRSGP` is **82 bytes owned by
`TokenkegQ…`** — a classic SPL mint with no Token-2022 extensions, so no transfer
fee hook exists. Whatever form the 2% takes, it is applied at the SOL pair or in
the bridge, not silently per swap. Freeze authority is disabled; mint authority
is the bridge program's `mintAuthorityPda`, which is how in-game gold becomes
tokens.

So the position today cuts both ways, and the second half still matters more:

* Nothing can be cashed out **yet**. All profit is GP-denominated, and the alch
  loop *mints* GP, so it dilutes the very thing it accumulates.
* Nothing has to be paid in either. Entry costs transaction fees and nothing
  else, so accumulation here is close to free optionality on the game becoming
  something people pay for — and an announced SOL pair is exactly the event that
  optionality is written against.

Either way the asset worth holding is the **scarce** one, not GP: an admin-funded
GP/SOL pool prices GP against a supply the bridge can mint. That is why the
pipeline ends in `accumulate` rather than in a GP balance.

### Measuring scarcity

`mercbot scarcity` makes this a measurement rather than a hunch, on two facts:

* Every pool was seeded with exactly **100 units**, so supply above 100 is what
  players have bridged out — direct evidence of how farmable an item is.
* **62 of the 1,374 registry items have no in-game source at all**: no drop
  table, no shop stock, no skill script, no quest reward, no ground spawn. Their
  supply cannot grow. The table is generated by `gamebot/tools/build-item-sources.ts`
  from the game's own content scripts.

```
$ mercbot scarcity --limit 6
market                    supply   bridged  score        price       buy GP  source
christmas_cracker            100         0   1.00     11014261     11243487  no in-game source
red_partyhat                 100         0   1.00     11014261     11243487  no in-game source
santa_hat                    100         0   1.00      2753595      2810906  no in-game source
halloweenmask_red            100         0   1.00      1101443      1124368  no in-game source
```

Two things the scanner taught us, both now encoded:

* **Scarcity is necessary, not sufficient.** 40 of the 62 unprintable items are
  macro-event cubes and quest litter floored at 0.9 GP. Ranking ties by *cheapest*
  put exactly that junk on top; ranking by the pool floor — the protocol's own
  valuation — puts party hats there instead.
* **Live supply beats the static table.** `blue_partyhat` has 199 units against a
  100-unit seed, so something produces it whatever the content scripts say. The
  accumulator scores it near zero and leaves it alone.

## Two things about this market that cost money to learn

**The floor is a price, not a bid.** Pools are seeded single-sided, so the GP a
pool can pay out is only what earlier buyers put in. A pool sitting exactly on its
floor holds *no GP at all* — you cannot sell into it at any size. `mercbot markets`
prints that as `bid depth`, and `risk.min_exit_depth_mult` will refuse buys
without it if the pool is the only exit you are willing to trust.

**Pools are shallow.** A hundred items each, so a five-item buy moves the price
about 7%. Set `risk.max_price_impact_pct` accordingly — too tight a cap silently
stops the bot trading at all — and let `alch-scan` pick the size where buying one
more item costs more than alching it returns.

## Correctness

The swap math is a port of the `cp_amm` program's own, and is checked against
reality rather than against itself:

* **Instruction layout** — verified against a real on-chain swap: same 14 accounts
  in the same order, same discriminator, same argument encoding.
* **Quote math** — verified against the reference `@meteora-ag/cp-amm-sdk` on four
  live mainnet pools. 40 quotes agree to the base unit, and both refuse the same
  12 impossible trades. The vectors are checked in, so the test runs offline.
* **Pool layout** — byte offsets derived from the IDL's C layout and pinned by a
  decode test over real account data.

`cargo test --workspace` runs 118 tests, none of which need a network.

## A caveat worth stating plainly

Alchemy mints GP from nothing, and withdrawing it mints GP tokens (or drains the
bridge's 1B vault). A successful alch loop is therefore *inflationary*, and its
profits are denominated in the thing it is inflating. The way to keep value is to
cycle GP back into items, whose supply is bounded by what pools hold and what
players bridge in — which is exactly what running `alch-arb` continuously does.

## Licence

MIT, matching Mercantile and rs-sdk.

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
gamebot/campaign.ts     equip -> stock runes -> train -> alch -> cash out GP
                                       |                            |
                        buys runes from Aubury/Betty      Exchange Clerk -> wallet
                                                                      |
mercbot run                                       accumulate: GP -> capped-supply items
```

`campaign.ts` derives its stage from what it observes rather than from saved
state, so it is safe to kill, restart, point at a half-trained account, or run
across a fleet at different stages with one command line.

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
| `alch-arb` | Buys only stacks High Level Alchemy would pay for. Exits through the game, not the pool. |
| `alch-floor` | Buys near the permanent floor, sells into strength. |
| `mean-reversion` | Fades moves away from a rolling mean, gated on the floor premium. |
| `grid` | A ladder anchored to the floor — the one price here that never drifts. |

`mercbot strategies` describes them; `mercbot.example.toml` configures them.

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

**Not today.** Checked rather than assumed: DexScreener lists 30 pairs for the GP
mint and **every one is item/GP** — zero pairs price GP against anything, zero
have a USD price, zero have USD liquidity. Jupiter returns `NO_ROUTES_FOUND` for
GP to SOL. The economy is closed.

That cuts both ways, and the second half matters more:

* Nothing can be cashed out. All profit is GP-denominated, and the alch loop
  *mints* GP, so it dilutes the very thing it accumulates.
* Nothing has to be paid in either. Entry costs transaction fees and nothing
  else, so accumulation here is close to free optionality on the game becoming
  something people pay for.

If a fiat bridge ever appears it will be a permissionless GP/SOL pool someone
chooses to fund, or peer-to-peer sales of the tokens themselves. Either way the
asset worth holding is the **scarce** one — which is why the pipeline ends in
`accumulate` rather than in a GP balance.

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

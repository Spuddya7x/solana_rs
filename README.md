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
| `gamebot/` | The in-game half. See [`gamebot/README.md`](gamebot/README.md). |

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
| `alch-arb` | Buys only stacks High Level Alchemy would pay for. Exits through the game, not the pool. |
| `alch-floor` | Buys near the permanent floor, sells into strength. |
| `mean-reversion` | Fades moves away from a rolling mean, gated on the floor premium. |
| `grid` | A ladder anchored to the floor — the one price here that never drifts. |

`mercbot strategies` describes them; `mercbot.example.toml` configures them.

## Getting an account to level 55

The bot that buys is useless without an account that can alch. That is
`gamebot/`'s job, and the route is short:

| Range | Method | Cost |
|---|---|---|
| 1 → 21 | Splash Confuse/Weaken/Curse on a chicken in a bronze kit | ~15 minutes, ~6,000 GP of runes |
| 21 → 55 | Low alchemy on floor-priced items (`0.4 × cost` out, `0.36 × cost` in) | funds itself |
| 55 → 66 | High alchemy, which is the business anyway | profitable throughout |

Splashing works because the engine pays spell XP *before* the hit roll, and
because a landed stat-reduction spell debuffs the target and blocks the next
cast — so missing is what keeps the grind going. A −64 magic attack bonus
guarantees the miss, and a full bronze kit is −69 for 399 GP.

Level 66 matters more than 55: it is the Wizards' Guild door, and the guild is
the only unbounded nature rune supply in the game. On-chain rune pools hold 100
units, which is about 20 casts before impact bites.

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

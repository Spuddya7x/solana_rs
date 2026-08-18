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
| `train-magic.ts` | Trains Magic to a target level, picking the best method available. |
| `alch-loop.ts` | Alchs a stack of items and optionally withdraws the GP to the wallet. |
| `lib/spells.ts` | Spell component ids, levels, runes, XP — derived from the game's config. |
| `lib/alch.ts` | Spell-on-item casting with XP-based completion detection. |
| `lib/bridge.ts` | Exchange Clerk dialogue: claim, withdraw GP, withdraw items. |
| `lib/economics.ts` | Alch values and break-evens, mirroring the Rust `alch` module. |

## The training route, and why it is nearly free

| Range | Method | Why |
|---|---|---|
| 1 → 21 | Combat spells on a chicken | 5,018 XP. Slow, costs runes, unavoidable. Splashing still trains — Magic XP is granted whether or not the spell hits. |
| 21 → 55 | **Low Level Alchemy** | 31 XP a cast at **3 ticks** — the fastest cast in the game — and it pays `0.4 × cost` for an item that cost `0.36 × cost` at the pool floor. Training from here **pays for itself**, and on expensive items it profits outright. |
| 55+ | **High Level Alchemy** | 65 XP and `0.6 × cost`. 5 ticks a cast, so 1,200 casts an hour. |

161,142 XP separates 21 from 55: about 5,200 low alchs, roughly two and a half
hours of casting. Feed it with `mercbot`:

```sh
mercbot alch-scan --limit 20                       # pick a target
mercbot run --once                                 # buy near the floor
# bridge in through the Exchange Clerk, then:
bun bots/alchbot/train-magic.ts --target 55 --alch "rune platebody"
bun bots/alchbot/alch-loop.ts --alch "rune platebody" --claim --withdraw-gp
```

## Runes

One nature rune per cast, and five fire runes unless a **staff of fire** is
equipped — buy the staff once and the fire runes stop mattering.

Nature runes are the recurring cost, and the supply routes are worth knowing:

* **On chain.** Nature runes are a tokenised item like any other, floor `7.2 GP`.
  Buy them with `mercbot` and bridge them in. This is the only route that needs
  no levels at all, and the reason the loop closes.
* **Wizards' Guild (Yanille).** 1,000 in stock, restocking — but the door checks
  for **66 Magic**, which is well past the 55 the spell itself needs.
* **Runecrafting.** Level 44, and a different grind entirely.

At 7 GP a rune, an item pays for its own cast above roughly 30 GP of shop cost,
so the constraint is not the runes — it is pool inventory. Each pool holds about
a hundred items, and buying more than about a fifth of one moves the price enough
to eat the margin. `mercbot alch-scan` sizes that for you.

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

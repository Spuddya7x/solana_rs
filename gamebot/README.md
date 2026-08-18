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

| Range | Method | XP/cast | Why |
|---|---|---|---|
| 1 → 3 | Wind Strike | 5.5 | 174 XP, ~32 casts. The only wasted minute in the plan. |
| 3 → 21 | **Splash** Confuse → Weaken → Curse | 13 / 21 / 29 | See below. ~250 casts, about **15 minutes**, ~6,000 GP of runes. |
| 21 → 55 | **Low Level Alchemy** | 31 | 3 ticks — the fastest cast in the game — and it pays `0.4 × cost` for an item that cost `0.36 × cost` at the pool floor. This leg **funds itself**. |
| 55 → 66 | **High Level Alchemy** | 65 | Every cast is already profitable, and 66 opens the Wizards' Guild — see *Runes*. |

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

* **On chain.** Nature runes are a tokenised item like any other, floor `7.2 GP`
  — but every pool is seeded with just **100 units**, runes included, so this is
  a trickle, not a supply. Live right now: 20 runes cost 11.3 GP each (29% price
  impact) and 60 cost 25.8 each (197%). Good for bootstrapping ~20 casts at a
  time; useless for running a business.
* **Wizards' Guild (Yanille).** 1,000 in stock, restocking, and the *only*
  unbounded source — but the door checks for **66 Magic**. That is 330,094 XP
  past level 55, about 5,100 high alchs, and every one of them already pays. It
  is the real target.
* **Runecrafting.** Level 44, and a different grind entirely.

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

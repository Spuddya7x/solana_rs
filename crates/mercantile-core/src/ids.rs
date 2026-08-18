//! Well-known Mercantile / Meteora addresses and market-wide constants.
//!
//! Sourced from the Mercantile repository (`chain/registry/registry.json`,
//! `chain/exchange/lib/constants.ts`) and the Meteora `cp_amm` IDL.

use solana_sdk::pubkey::Pubkey;

/// Meteora DAMM v2 (`cp_amm`) program — every item/GP pool lives here.
pub const CP_AMM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");

/// `cp_amm` pool authority PDA (`["pool_authority"]`), a fixed account on every swap.
pub const CP_AMM_POOL_AUTHORITY: Pubkey =
    Pubkey::from_str_const("HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC");

/// Mercantile's bridge program: mints on withdraw (game to chain), burns on deposit.
/// The bot never calls it — it is here so tooling can label bridge activity.
pub const MERCANTILE_BRIDGE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("H5C6RKWQzUdfS8tVzZb3uVcRw3EHCzghv7kWdMBTD2bS");

/// Gielinor GP — the quote asset of every market.
pub const GP_MINT: Pubkey = Pubkey::from_str_const("123B7bdJzDYGkrAg7i3JUi5TaHYP47dqmSiR5qPRSGP");

/// GP is a 6-decimal SPL token.
pub const GP_DECIMALS: u8 = 6;

/// Item tokens carry 1 decimal (0 decimals would trip wallets' "this is an NFT"
/// heuristic), so one whole item is 10 base units.
pub const ITEM_DECIMALS: u8 = 1;

/// Base units in one whole item.
pub const ITEM_UNIT: u64 = 10u64.pow(ITEM_DECIMALS as u32);

/// Base units in one GP.
pub const GP_UNIT: u64 = 10u64.pow(GP_DECIMALS as u32);

/// Canonical registry, tracking the Mercantile repository's `main` branch.
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/MidTermDev/mercantile/main/chain/registry/registry.json";

/// Public mainnet RPC. Rate-limited — point the bot at a paid endpoint for real use.
pub const DEFAULT_MAINNET_RPC: &str = "https://api.mainnet-beta.solana.com";

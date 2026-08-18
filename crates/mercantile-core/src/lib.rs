//! Core primitives for the [Mercantile](https://github.com/MidTermDev/mercantile) economy:
//! the item registry, the on-chain addresses that define the market, and the
//! amount/price conversions that everything else is expressed in.
//!
//! Mercantile tokenises every tradeable item of a 2004-era RuneScape world on
//! Solana. Each item is an SPL mint paired against **GP** (the tokenised in-game
//! currency) in a Meteora DAMM v2 pool. This crate knows *what* the markets are;
//! [`mercantile_dex`](../mercantile_dex/index.html) knows how to price and trade them.

pub mod alch;
pub mod amounts;
pub mod ids;
pub mod registry;

pub use alch::{high_alch_value, low_alch_value, HIGH_ALCH_LEVEL, HIGH_ALCH_MULT, LOW_ALCH_MULT};
pub use amounts::{
    base_to_gp, base_to_items, gp_to_base, items_to_base, price_from_sqrt_price,
    sqrt_price_from_price,
};
pub use ids::{
    CP_AMM_POOL_AUTHORITY, CP_AMM_PROGRAM_ID, DEFAULT_MAINNET_RPC, DEFAULT_REGISTRY_URL,
    GP_DECIMALS, GP_MINT, ITEM_DECIMALS, MERCANTILE_BRIDGE_PROGRAM_ID,
};
pub use registry::{Item, Market, Registry, RegistryError};

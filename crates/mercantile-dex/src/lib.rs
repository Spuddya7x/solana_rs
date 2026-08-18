//! Meteora DAMM v2 (`cp_amm`) access for Mercantile's item/GP pools.
//!
//! Every Mercantile item is token A of a concentrated-liquidity pool whose token B
//! is GP. This crate decodes those pool accounts, reproduces the program's swap
//! math exactly (so quotes match what a transaction would actually do), builds the
//! swap instructions, and wraps the RPC calls the bot needs.
//!
//! The math is a port of the `@meteora-ag/cp-amm-sdk` quote path for
//! `CollectFeeMode::OnlyB` concentrated-liquidity pools — the configuration every
//! Mercantile pool uses (flat 1% fee collected in GP, range `[P0, MAX)`).

pub mod client;
pub mod fees;
pub mod math;
pub mod pool;
pub mod quote;
pub mod swap;

pub use client::{ChainClient, PoolFetch};
pub use pool::{CollectFeeMode, PoolState, TradeDirection};
pub use quote::{Quote, Side, SwapQuote};
pub use swap::{swap_instruction, SwapMode, SwapParams};

/// Errors from decoding, quoting or building swaps.
#[derive(Debug, thiserror::Error)]
pub enum DexError {
    #[error("account is {len} bytes, not a cp-amm pool ({expected} expected)")]
    BadAccountLength { len: usize, expected: usize },
    #[error("account discriminator {found:02x?} is not a cp-amm pool")]
    BadDiscriminator { found: [u8; 8] },
    #[error("pool is disabled for swaps")]
    PoolDisabled,
    #[error("pool has no liquidity")]
    NoLiquidity,
    #[error("swap amount is zero")]
    ZeroAmount,
    #[error("swap would push the price outside the pool's range [{min}, {max}]")]
    PriceRangeViolation { min: u128, max: u128 },
    #[error("insufficient liquidity to fill the requested output")]
    InsufficientLiquidity,
    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),
    #[error(
        "unsupported base fee mode {0} — this pool prices fees in a way the bot does not model"
    )]
    UnsupportedFeeMode(u8),
    #[error("unsupported collect fee mode {0}")]
    UnsupportedCollectFeeMode(u8),
    #[error("pool is not activated yet (activation point {activation_point}, now {now})")]
    NotActivated { activation_point: u64, now: u64 },
    #[error(transparent)]
    Rpc(#[from] Box<solana_client::client_error::ClientError>),
}

pub type Result<T> = std::result::Result<T, DexError>;

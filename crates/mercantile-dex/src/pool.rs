//! Decoding of the `cp_amm` `Pool` account.
//!
//! The account is a bytemuck `#[repr(C)]` struct behind an 8-byte Anchor
//! discriminator, so field positions are fixed byte offsets rather than a Borsh
//! stream. The offsets below were derived from the IDL's C layout and verified
//! against a live mainnet pool (see `tests/pool_decode.rs`); `Pool::LEN` is a
//! hard check that the layout has not shifted under us.

use solana_sdk::pubkey::Pubkey;

use crate::{DexError, Result};

/// Anchor discriminator for the `Pool` account.
pub const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];

/// Where the pool's fees are taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectFeeMode {
    /// Fees in whichever token is the output.
    BothToken,
    /// Fees always in token B (GP) — what every Mercantile pool uses.
    OnlyB,
    /// Fees compounded back into the pool.
    Compounding,
}

impl CollectFeeMode {
    fn from_u8(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::BothToken),
            1 => Ok(Self::OnlyB),
            2 => Ok(Self::Compounding),
            other => Err(DexError::UnsupportedCollectFeeMode(other)),
        }
    }
}

/// Which way a swap runs through the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeDirection {
    /// Item in, GP out — selling an item.
    AtoB,
    /// GP in, item out — buying an item.
    BtoA,
}

/// Whether the fee is charged on the way in or on the way out, and to whom.
#[derive(Debug, Clone, Copy)]
pub struct FeeMode {
    pub fees_on_input: bool,
    pub has_referral: bool,
}

impl FeeMode {
    /// Mirrors the program's `get_fee_mode`.
    pub fn resolve(
        collect_fee_mode: CollectFeeMode,
        direction: TradeDirection,
        has_referral: bool,
    ) -> Self {
        let fees_on_input = matches!(
            (collect_fee_mode, direction),
            (CollectFeeMode::OnlyB, TradeDirection::BtoA)
                | (CollectFeeMode::Compounding, TradeDirection::BtoA)
        );
        Self {
            fees_on_input,
            has_referral,
        }
    }
}

/// The dynamic (volatility-driven) fee component. Disabled on Mercantile pools.
#[derive(Debug, Clone, Copy)]
pub struct DynamicFee {
    pub initialized: bool,
    pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
    pub bin_step: u16,
    pub volatility_accumulator: u128,
}

/// The pool's fee configuration, as far as quoting needs it.
#[derive(Debug, Clone, Copy)]
pub struct PoolFees {
    /// Raw 32-byte pod-aligned base-fee blob; interpreted by [`crate::fees`].
    pub base_fee_info: [u8; 32],
    pub protocol_fee_percent: u8,
    pub referral_fee_percent: u8,
    pub compounding_fee_bps: u16,
    pub dynamic_fee: DynamicFee,
    pub init_sqrt_price: u128,
}

/// A decoded `cp_amm` pool.
#[derive(Debug, Clone)]
pub struct PoolState {
    pub fees: PoolFees,
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub liquidity: u128,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub sqrt_price: u128,
    pub activation_point: u64,
    /// 0 = slot, 1 = unix timestamp. Mercantile pools activate by timestamp.
    pub activation_type: u8,
    /// 0 = enabled, 1 = disabled.
    pub pool_status: u8,
    pub token_a_flag: u8,
    pub token_b_flag: u8,
    pub collect_fee_mode: CollectFeeMode,
    pub fee_version: u8,
    pub token_a_amount: u64,
    pub token_b_amount: u64,
}

macro_rules! read {
    ($buf:expr, $offset:expr, u8) => {
        $buf[$offset]
    };
    ($buf:expr, $offset:expr, u16) => {
        u16::from_le_bytes($buf[$offset..$offset + 2].try_into().unwrap())
    };
    ($buf:expr, $offset:expr, u32) => {
        u32::from_le_bytes($buf[$offset..$offset + 4].try_into().unwrap())
    };
    ($buf:expr, $offset:expr, u64) => {
        u64::from_le_bytes($buf[$offset..$offset + 8].try_into().unwrap())
    };
    ($buf:expr, $offset:expr, u128) => {
        u128::from_le_bytes($buf[$offset..$offset + 16].try_into().unwrap())
    };
    ($buf:expr, $offset:expr, pubkey) => {
        Pubkey::new_from_array($buf[$offset..$offset + 32].try_into().unwrap())
    };
}

impl PoolState {
    /// Size of the `Pool` struct after the discriminator.
    pub const STRUCT_LEN: usize = 1104;
    /// Total account size including the discriminator.
    pub const LEN: usize = Self::STRUCT_LEN + 8;

    /// Decode a pool from raw account data.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() != Self::LEN {
            return Err(DexError::BadAccountLength {
                len: data.len(),
                expected: Self::LEN,
            });
        }
        let discriminator: [u8; 8] = data[..8].try_into().unwrap();
        if discriminator != POOL_DISCRIMINATOR {
            return Err(DexError::BadDiscriminator {
                found: discriminator,
            });
        }
        let b = &data[8..];

        let dynamic_fee = DynamicFee {
            initialized: read!(b, 48, u8) != 0,
            max_volatility_accumulator: read!(b, 56, u32),
            variable_fee_control: read!(b, 60, u32),
            bin_step: read!(b, 64, u16),
            volatility_accumulator: read!(b, 112, u128),
        };

        Ok(Self {
            fees: PoolFees {
                base_fee_info: b[0..32].try_into().unwrap(),
                protocol_fee_percent: read!(b, 40, u8),
                referral_fee_percent: read!(b, 42, u8),
                compounding_fee_bps: read!(b, 46, u16),
                dynamic_fee,
                init_sqrt_price: read!(b, 144, u128),
            },
            token_a_mint: read!(b, 160, pubkey),
            token_b_mint: read!(b, 192, pubkey),
            token_a_vault: read!(b, 224, pubkey),
            token_b_vault: read!(b, 256, pubkey),
            liquidity: read!(b, 352, u128),
            sqrt_min_price: read!(b, 416, u128),
            sqrt_max_price: read!(b, 432, u128),
            sqrt_price: read!(b, 448, u128),
            activation_point: read!(b, 464, u64),
            activation_type: read!(b, 472, u8),
            pool_status: read!(b, 473, u8),
            token_a_flag: read!(b, 474, u8),
            token_b_flag: read!(b, 475, u8),
            collect_fee_mode: CollectFeeMode::from_u8(read!(b, 476, u8))?,
            fee_version: read!(b, 478, u8),
            token_a_amount: read!(b, 672, u64),
            token_b_amount: read!(b, 680, u64),
        })
    }

    /// Whether the program will accept swaps against this pool right now.
    pub fn is_swap_enabled(&self, current_point: u64) -> bool {
        self.pool_status == 0 && current_point >= self.activation_point
    }

    /// Spot price in GP per whole item.
    pub fn spot_price(&self) -> f64 {
        mercantile_core::price_from_sqrt_price(self.sqrt_price)
    }

    /// The pool's permanent bid floor in GP per whole item — its seed price,
    /// which is also the bottom of its liquidity range.
    pub fn floor_price(&self) -> f64 {
        mercantile_core::price_from_sqrt_price(self.sqrt_min_price)
    }

    /// How far above the floor the pool is trading, as a multiple (1.0 == at the floor).
    pub fn premium_over_floor(&self) -> f64 {
        let floor = self.floor_price();
        if floor <= 0.0 {
            return f64::NAN;
        }
        self.spot_price() / floor
    }

    /// Item base units the pool can still sell before the price runs to its ceiling.
    pub fn item_reserve(&self) -> Result<u128> {
        crate::math::amount_a_from_liquidity(
            self.sqrt_price,
            self.sqrt_max_price,
            self.liquidity,
            crate::math::Rounding::Down,
        )
    }

    /// GP base units the pool can still pay out before the price reaches its floor.
    pub fn gp_reserve(&self) -> Result<u128> {
        crate::math::amount_b_from_liquidity(
            self.sqrt_min_price,
            self.sqrt_price,
            self.liquidity,
            crate::math::Rounding::Down,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_sized_accounts() {
        let err = PoolState::decode(&[0u8; 32]).unwrap_err();
        assert!(matches!(err, DexError::BadAccountLength { .. }), "{err}");
    }

    #[test]
    fn rejects_foreign_discriminators() {
        let mut data = vec![0u8; PoolState::LEN];
        data[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let err = PoolState::decode(&data).unwrap_err();
        assert!(matches!(err, DexError::BadDiscriminator { .. }), "{err}");
    }

    #[test]
    fn fee_mode_matches_the_program() {
        // OnlyB: buying (GP in) pays the fee on input, selling pays it on output.
        let buy = FeeMode::resolve(CollectFeeMode::OnlyB, TradeDirection::BtoA, false);
        let sell = FeeMode::resolve(CollectFeeMode::OnlyB, TradeDirection::AtoB, false);
        assert!(buy.fees_on_input);
        assert!(!sell.fees_on_input);
        // BothToken always charges on output.
        for direction in [TradeDirection::AtoB, TradeDirection::BtoA] {
            assert!(!FeeMode::resolve(CollectFeeMode::BothToken, direction, false).fees_on_input);
        }
    }
}

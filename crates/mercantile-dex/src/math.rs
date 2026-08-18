//! Fixed-point helpers shared by the quote path.
//!
//! Prices are Q64.64 sqrt prices and liquidity is a u128, so intermediate products
//! routinely exceed 128 bits (a typical Mercantile pool carries liquidity around
//! 2^84). Everything here therefore computes in 512 bits and narrows once, at the
//! end, with a checked conversion — the same shape as the on-chain program, so the
//! rounding matches instruction-for-instruction.

use primitive_types::U512;

use crate::{DexError, Result};

/// Q64.64 scale exponent.
pub const SCALE_OFFSET: u32 = 64;
/// Fee numerators are expressed out of 1e9.
pub const FEE_DENOMINATOR: u128 = 1_000_000_000;
/// 100% in basis points.
pub const BASIS_POINT_MAX: u128 = 10_000;
/// Lowest sqrt price the program will accept.
pub const MIN_SQRT_PRICE: u128 = 4_295_048_016;
/// Highest sqrt price the program will accept — Mercantile pools use this as their upper bound.
pub const MAX_SQRT_PRICE: u128 = 79_226_673_521_066_979_257_578_248_091;
/// Max fee numerator for pools with `fee_version == 0` (50%).
pub const MAX_FEE_NUMERATOR_V0: u128 = 500_000_000;
/// Max fee numerator for pools with `fee_version == 1` (99%).
pub const MAX_FEE_NUMERATOR_V1: u128 = 990_000_000;

/// Rounding direction for [`mul_div`], matching the program's `Rounding` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    Up,
    Down,
}

/// `1 << 64` as Q64.64 one.
pub fn one_q64() -> U512 {
    U512::one() << SCALE_OFFSET
}

/// `x * y / denominator` in 512-bit space with explicit rounding.
pub fn mul_div(x: U512, y: U512, denominator: U512, rounding: Rounding) -> Result<U512> {
    if denominator.is_zero() {
        return Err(DexError::Overflow("mul_div by zero"));
    }
    let product = x
        .checked_mul(y)
        .ok_or(DexError::Overflow("mul_div product"))?;
    let quotient = product / denominator;
    if rounding == Rounding::Up && !(product % denominator).is_zero() {
        quotient
            .checked_add(U512::one())
            .ok_or(DexError::Overflow("mul_div round up"))
    } else {
        Ok(quotient)
    }
}

/// Narrow a 512-bit value to `u128`, erroring instead of truncating.
pub fn to_u128(value: U512, context: &'static str) -> Result<u128> {
    if value.bits() > 128 {
        return Err(DexError::Overflow(context));
    }
    Ok(value.low_u128())
}

/// Narrow a 512-bit value to `u64`, erroring instead of truncating.
pub fn to_u64(value: U512, context: &'static str) -> Result<u64> {
    if value.bits() > 64 {
        return Err(DexError::Overflow(context));
    }
    Ok(value.low_u64())
}

/// Token B (GP) implied by moving the price between two sqrt prices at a given liquidity.
///
/// `L * (upper - lower) / 2^128`
pub fn amount_b_from_liquidity(
    lower_sqrt_price: u128,
    upper_sqrt_price: u128,
    liquidity: u128,
    rounding: Rounding,
) -> Result<u128> {
    let delta = upper_sqrt_price
        .checked_sub(lower_sqrt_price)
        .ok_or(DexError::Overflow("amount_b delta"))?;
    let product = U512::from(liquidity)
        .checked_mul(U512::from(delta))
        .ok_or(DexError::Overflow("amount_b product"))?;
    let shift = SCALE_OFFSET * 2;
    let result = if rounding == Rounding::Up {
        let denominator = U512::one() << shift;
        (product + denominator - U512::one()) / denominator
    } else {
        product >> shift
    };
    to_u128(result, "amount_b result")
}

/// Token A (items) implied by moving the price between two sqrt prices at a given liquidity.
///
/// `L * (upper - lower) / (lower * upper)`
pub fn amount_a_from_liquidity(
    lower_sqrt_price: u128,
    upper_sqrt_price: u128,
    liquidity: u128,
    rounding: Rounding,
) -> Result<u128> {
    let numerator2 = upper_sqrt_price
        .checked_sub(lower_sqrt_price)
        .ok_or(DexError::Overflow("amount_a delta"))?;
    let denominator = U512::from(lower_sqrt_price)
        .checked_mul(U512::from(upper_sqrt_price))
        .ok_or(DexError::Overflow("amount_a denominator"))?;
    if denominator.is_zero() {
        return Err(DexError::Overflow("amount_a zero denominator"));
    }
    let result = mul_div(
        U512::from(liquidity),
        U512::from(numerator2),
        denominator,
        rounding,
    )?;
    to_u128(result, "amount_a result")
}

/// Next sqrt price after adding `amount` of token A (price falls).
///
/// `L * sqrt_price / (L + amount * sqrt_price)`, rounded up.
pub fn next_sqrt_price_from_amount_a_in(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
) -> Result<u128> {
    if amount == 0 {
        return Ok(sqrt_price);
    }
    let product = U512::from(amount)
        .checked_mul(U512::from(sqrt_price))
        .ok_or(DexError::Overflow("next_sqrt_price_a_in product"))?;
    let denominator = U512::from(liquidity)
        .checked_add(product)
        .ok_or(DexError::Overflow("next_sqrt_price_a_in denominator"))?;
    let result = mul_div(
        U512::from(liquidity),
        U512::from(sqrt_price),
        denominator,
        Rounding::Up,
    )?;
    to_u128(result, "next_sqrt_price_a_in")
}

/// Next sqrt price after adding `amount` of token B (price rises).
///
/// `sqrt_price + (amount << 128) / L`, rounded down.
pub fn next_sqrt_price_from_amount_b_in(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
) -> Result<u128> {
    if liquidity == 0 {
        return Err(DexError::NoLiquidity);
    }
    let quotient = (U512::from(amount) << (SCALE_OFFSET * 2)) / U512::from(liquidity);
    let result = quotient
        .checked_add(U512::from(sqrt_price))
        .ok_or(DexError::Overflow("next_sqrt_price_b_in"))?;
    to_u128(result, "next_sqrt_price_b_in")
}

/// Next sqrt price after removing `amount` of token A (price rises — a buy).
///
/// `L * sqrt_price / (L - amount * sqrt_price)`, rounded up.
pub fn next_sqrt_price_from_amount_a_out(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
) -> Result<u128> {
    if amount == 0 {
        return Ok(sqrt_price);
    }
    let product = U512::from(amount)
        .checked_mul(U512::from(sqrt_price))
        .ok_or(DexError::Overflow("next_sqrt_price_a_out product"))?;
    let liquidity = U512::from(liquidity);
    if product >= liquidity {
        // The pool cannot source that many items at any price.
        return Err(DexError::InsufficientLiquidity);
    }
    let denominator = liquidity - product;
    let result = mul_div(liquidity, U512::from(sqrt_price), denominator, Rounding::Up)?;
    to_u128(result, "next_sqrt_price_a_out")
}

/// Next sqrt price after removing `amount` of token B (price falls — a sell).
///
/// `sqrt_price - ceil((amount << 128) / L)`.
pub fn next_sqrt_price_from_amount_b_out(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
) -> Result<u128> {
    if liquidity == 0 {
        return Err(DexError::NoLiquidity);
    }
    let numerator = U512::from(amount) << (SCALE_OFFSET * 2);
    let liquidity = U512::from(liquidity);
    let quotient = (numerator + liquidity - U512::one()) / liquidity;
    let sqrt_price = U512::from(sqrt_price);
    if quotient > sqrt_price {
        return Err(DexError::InsufficientLiquidity);
    }
    to_u128(sqrt_price - quotient, "next_sqrt_price_b_out")
}

/// Q64.64 exponentiation by squaring, used by the exponential fee scheduler.
pub fn pow_q64(base: U512, exponent: u32) -> Result<U512> {
    let one = one_q64();
    if exponent == 0 {
        return Ok(one);
    }
    let mut result = one;
    let mut squared = base;
    let mut exp = exponent;
    while exp > 0 {
        if exp & 1 == 1 {
            result = (result
                .checked_mul(squared)
                .ok_or(DexError::Overflow("pow_q64 mul"))?)
                >> SCALE_OFFSET;
        }
        exp >>= 1;
        if exp > 0 {
            squared = (squared
                .checked_mul(squared)
                .ok_or(DexError::Overflow("pow_q64 square"))?)
                >> SCALE_OFFSET;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_rounds_both_ways() {
        let (x, y, d) = (U512::from(7u8), U512::from(3u8), U512::from(2u8));
        assert_eq!(mul_div(x, y, d, Rounding::Down).unwrap(), U512::from(10u8));
        assert_eq!(mul_div(x, y, d, Rounding::Up).unwrap(), U512::from(11u8));
    }

    #[test]
    fn amount_helpers_survive_realistic_magnitudes() {
        // Live 1dose1agility pool at slot 440_044_167.
        let liquidity = 24_748_911_958_502_412_996_944_667u128;
        let sqrt_price = 27_169_186_055_242_833_301_691u128;
        let sqrt_min = 24_748_904_227_413_569_152_000u128;
        // Products here are ~2^165, so this only works because we compute in 512 bits.
        let gp = amount_b_from_liquidity(sqrt_min, sqrt_price, liquidity, Rounding::Down).unwrap();
        assert!(gp > 0);
        let items =
            amount_a_from_liquidity(sqrt_price, MAX_SQRT_PRICE, liquidity, Rounding::Up).unwrap();
        assert!(items > 0);
    }

    #[test]
    fn adding_token_b_raises_the_price_and_a_lowers_it() {
        let liquidity = 24_748_911_958_502_412_996_944_667u128;
        let sqrt_price = 27_169_186_055_242_833_301_691u128;
        let up = next_sqrt_price_from_amount_b_in(sqrt_price, liquidity, 1_000_000).unwrap();
        let down = next_sqrt_price_from_amount_a_in(sqrt_price, liquidity, 10).unwrap();
        assert!(up > sqrt_price);
        assert!(down < sqrt_price);
    }

    #[test]
    fn oversized_output_reports_insufficient_liquidity() {
        let err = next_sqrt_price_from_amount_a_out(1 << 64, 1_000, u128::MAX / 2).unwrap_err();
        assert!(matches!(err, DexError::InsufficientLiquidity), "{err}");
    }

    #[test]
    fn pow_q64_matches_repeated_multiplication() {
        let base = one_q64() - (one_q64() >> 4); // 0.9375
        let expected = {
            let mut acc = one_q64();
            for _ in 0..5 {
                acc = (acc * base) >> SCALE_OFFSET;
            }
            acc
        };
        assert_eq!(pow_q64(base, 5).unwrap(), expected);
        assert_eq!(pow_q64(base, 0).unwrap(), one_q64());
    }
}

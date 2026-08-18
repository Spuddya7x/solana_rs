//! The pool's trading fee.
//!
//! A `cp_amm` pool's base fee is a 32-byte pod-aligned blob whose first byte after
//! the cliff numerator selects a scheduler. Mercantile pools use a **fee time
//! scheduler with zero periods**, i.e. a flat 1% (cliff numerator 10_000_000 out of
//! 1e9) collected in GP — but the scheduler is implemented properly here so a
//! repriced or newly created pool with a real schedule still quotes correctly.
//!
//! The rate-limiter and market-cap schedulers are deliberately *not* modelled:
//! their fee depends on trade size and price history, and guessing would produce
//! quotes that silently disagree with the chain. Those pools error out instead.

use primitive_types::U512;

use crate::math::{
    one_q64, pow_q64, to_u128, BASIS_POINT_MAX, FEE_DENOMINATOR, MAX_FEE_NUMERATOR_V0,
    MAX_FEE_NUMERATOR_V1, SCALE_OFFSET,
};
use crate::pool::PoolFees;
use crate::{DexError, Result};

/// Base-fee scheduler variants, as encoded in the pod-aligned blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseFeeMode {
    FeeTimeSchedulerLinear = 0,
    FeeTimeSchedulerExponential = 1,
    RateLimiter = 2,
    FeeMarketCapSchedulerLinear = 3,
    FeeMarketCapSchedulerExponential = 4,
}

/// A decoded fee time scheduler.
#[derive(Debug, Clone, Copy)]
pub struct FeeTimeScheduler {
    pub cliff_fee_numerator: u64,
    pub mode: BaseFeeMode,
    pub number_of_period: u16,
    pub period_frequency: u64,
    pub reduction_factor: u64,
}

impl FeeTimeScheduler {
    /// Decode the pod-aligned base-fee blob.
    ///
    /// Layout: `cliff_fee_numerator: u64`, `base_fee_mode: u8`, 5 bytes padding,
    /// `number_of_period: u16`, `period_frequency: u64`, `reduction_factor: u64`.
    pub fn decode(data: &[u8; 32]) -> Result<Self> {
        let mode = match data[8] {
            0 => BaseFeeMode::FeeTimeSchedulerLinear,
            1 => BaseFeeMode::FeeTimeSchedulerExponential,
            other => return Err(DexError::UnsupportedFeeMode(other)),
        };
        Ok(Self {
            cliff_fee_numerator: u64::from_le_bytes(data[0..8].try_into().unwrap()),
            mode,
            number_of_period: u16::from_le_bytes(data[14..16].try_into().unwrap()),
            period_frequency: u64::from_le_bytes(data[16..24].try_into().unwrap()),
            reduction_factor: u64::from_le_bytes(data[24..32].try_into().unwrap()),
        })
    }

    /// The base fee numerator in force at `current_point`.
    ///
    /// Before activation the schedule is treated as fully elapsed (the program's
    /// behaviour: a pre-activation quote sees the final, lowest fee).
    pub fn base_fee_numerator(&self, current_point: u64, activation_point: u64) -> Result<u128> {
        if self.period_frequency == 0 {
            return Ok(self.cliff_fee_numerator as u128);
        }
        let period = if current_point < activation_point {
            self.number_of_period as u64
        } else {
            ((current_point - activation_point) / self.period_frequency)
                .min(self.number_of_period as u64)
        };
        self.fee_numerator_at_period(period as u32)
    }

    fn fee_numerator_at_period(&self, period: u32) -> Result<u128> {
        let cliff = self.cliff_fee_numerator as u128;
        match self.mode {
            BaseFeeMode::FeeTimeSchedulerLinear => {
                let reduction = (period as u128)
                    .checked_mul(self.reduction_factor as u128)
                    .ok_or(DexError::Overflow("linear fee reduction"))?;
                Ok(cliff.saturating_sub(reduction))
            }
            BaseFeeMode::FeeTimeSchedulerExponential => {
                if period == 0 {
                    return Ok(cliff);
                }
                let bps = (U512::from(self.reduction_factor) << SCALE_OFFSET)
                    / U512::from(BASIS_POINT_MAX);
                let base = one_q64()
                    .checked_sub(bps)
                    .ok_or(DexError::Overflow("exponential fee base"))?;
                let factor = pow_q64(base, period)?;
                let result = (factor
                    .checked_mul(U512::from(cliff))
                    .ok_or(DexError::Overflow("exponential fee"))?)
                    >> SCALE_OFFSET;
                to_u128(result, "exponential fee numerator")
            }
            other => Err(DexError::UnsupportedFeeMode(other as u8)),
        }
    }
}

/// Highest fee numerator the pool version allows.
pub fn max_fee_numerator(fee_version: u8) -> u128 {
    match fee_version {
        0 => MAX_FEE_NUMERATOR_V0,
        _ => MAX_FEE_NUMERATOR_V1,
    }
}

/// Total trading fee numerator: base schedule plus any dynamic (volatility) fee,
/// capped at the pool version's maximum.
pub fn total_fee_numerator(
    fees: &PoolFees,
    fee_version: u8,
    current_point: u64,
    activation_point: u64,
) -> Result<u128> {
    let scheduler = FeeTimeScheduler::decode(&fees.base_fee_info)?;
    let base = scheduler.base_fee_numerator(current_point, activation_point)?;
    let dynamic = if fees.dynamic_fee.initialized {
        dynamic_fee_numerator(
            fees.dynamic_fee.volatility_accumulator,
            fees.dynamic_fee.bin_step as u128,
            fees.dynamic_fee.variable_fee_control as u128,
        )?
    } else {
        0
    };
    Ok(base
        .saturating_add(dynamic)
        .min(max_fee_numerator(fee_version)))
}

/// `variable_fee_control * (volatility_accumulator * bin_step)^2`, scaled down.
fn dynamic_fee_numerator(
    volatility_accumulator: u128,
    bin_step: u128,
    variable_fee_control: u128,
) -> Result<u128> {
    const SCALING_FACTOR: u128 = 100_000_000_000;
    const ROUNDING_OFFSET: u128 = 99_999_999_999;
    let vfa_bin = U512::from(volatility_accumulator)
        .checked_mul(U512::from(bin_step))
        .ok_or(DexError::Overflow("dynamic fee vfa"))?;
    let squared = vfa_bin
        .checked_mul(vfa_bin)
        .ok_or(DexError::Overflow("dynamic fee square"))?;
    let v_fee = U512::from(variable_fee_control)
        .checked_mul(squared)
        .ok_or(DexError::Overflow("dynamic fee product"))?;
    let result = (v_fee + U512::from(ROUNDING_OFFSET)) / U512::from(SCALING_FACTOR);
    to_u128(result, "dynamic fee numerator")
}

/// Split a trading fee into its protocol / referral / LP parts.
#[derive(Debug, Clone, Copy, Default)]
pub struct FeeSplit {
    pub lp_fee: u128,
    pub protocol_fee: u128,
    pub referral_fee: u128,
}

/// Mirrors the program's `split_fees`.
pub fn split_fees(fees: &PoolFees, fee_amount: u128, has_referral: bool) -> FeeSplit {
    let mut protocol_fee = fee_amount * fees.protocol_fee_percent as u128 / 100;
    let lp_fee = fee_amount - protocol_fee;
    let referral_fee = if has_referral {
        protocol_fee * fees.referral_fee_percent as u128 / 100
    } else {
        0
    };
    protocol_fee -= referral_fee;
    FeeSplit {
        lp_fee,
        protocol_fee,
        referral_fee,
    }
}

/// Take `fee_numerator` out of an amount: what is left, and the fee itself.
pub fn exclude_fee(fee_numerator: u128, included_fee_amount: u128) -> (u128, u128) {
    let trading_fee = included_fee_amount
        .saturating_mul(fee_numerator)
        .div_ceil(FEE_DENOMINATOR);
    (included_fee_amount - trading_fee, trading_fee)
}

/// Gross up an amount so that after `fee_numerator` is taken, the target remains.
pub fn include_fee(fee_numerator: u128, excluded_fee_amount: u128) -> Result<(u128, u128)> {
    let denominator = FEE_DENOMINATOR
        .checked_sub(fee_numerator)
        .filter(|d| *d > 0)
        .ok_or(DexError::Overflow("fee numerator exceeds denominator"))?;
    let included = excluded_fee_amount
        .checked_mul(FEE_DENOMINATOR)
        .ok_or(DexError::Overflow("include_fee product"))?
        .div_ceil(denominator);
    Ok((included, included - excluded_fee_amount))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{DynamicFee, PoolFees};

    /// The blob every Mercantile pool carries: flat 1%, no schedule.
    fn mercantile_base_fee() -> [u8; 32] {
        let mut data = [0u8; 32];
        data[0..8].copy_from_slice(&10_000_000u64.to_le_bytes());
        data
    }

    fn pool_fees(base: [u8; 32]) -> PoolFees {
        PoolFees {
            base_fee_info: base,
            protocol_fee_percent: 20,
            referral_fee_percent: 20,
            compounding_fee_bps: 0,
            dynamic_fee: DynamicFee {
                initialized: false,
                max_volatility_accumulator: 0,
                variable_fee_control: 0,
                bin_step: 0,
                volatility_accumulator: 0,
            },
            init_sqrt_price: 0,
        }
    }

    #[test]
    fn flat_one_percent_is_constant_over_time() {
        let fees = pool_fees(mercantile_base_fee());
        for point in [0u64, 1_787_019_263, u64::MAX / 2] {
            let numerator = total_fee_numerator(&fees, 1, point, 1_787_019_263).unwrap();
            assert_eq!(numerator, 10_000_000, "1% of 1e9");
        }
    }

    #[test]
    fn linear_schedule_decays_then_holds() {
        let mut data = [0u8; 32];
        data[0..8].copy_from_slice(&50_000_000u64.to_le_bytes()); // 5% cliff
        data[8] = 0; // linear
        data[14..16].copy_from_slice(&4u16.to_le_bytes()); // 4 periods
        data[16..24].copy_from_slice(&60u64.to_le_bytes()); // every 60s
        data[24..32].copy_from_slice(&10_000_000u64.to_le_bytes()); // -1% per period
        let scheduler = FeeTimeScheduler::decode(&data).unwrap();
        let activation = 1_000;
        assert_eq!(
            scheduler.base_fee_numerator(1_000, activation).unwrap(),
            50_000_000
        );
        assert_eq!(
            scheduler.base_fee_numerator(1_120, activation).unwrap(),
            30_000_000
        );
        // Clamped at number_of_period.
        assert_eq!(
            scheduler.base_fee_numerator(9_999, activation).unwrap(),
            10_000_000
        );
        // Before activation the schedule reads as fully elapsed.
        assert_eq!(
            scheduler.base_fee_numerator(0, activation).unwrap(),
            10_000_000
        );
    }

    #[test]
    fn unsupported_schedulers_are_refused_not_guessed() {
        let mut data = mercantile_base_fee();
        data[8] = 2; // rate limiter
        let err = FeeTimeScheduler::decode(&data).unwrap_err();
        assert!(matches!(err, DexError::UnsupportedFeeMode(2)), "{err}");
    }

    #[test]
    fn fee_inclusion_round_trips() {
        let numerator = 10_000_000; // 1%
        let (included, fee) = include_fee(numerator, 990_000).unwrap();
        let (excluded, fee2) = exclude_fee(numerator, included);
        assert!(excluded >= 990_000, "grossing up must not undershoot");
        assert_eq!(fee, included - 990_000);
        assert!(fee2 >= fee.saturating_sub(1));
    }

    #[test]
    fn protocol_share_is_taken_before_the_referral_share() {
        let fees = pool_fees(mercantile_base_fee());
        let split = split_fees(&fees, 1_000, false);
        assert_eq!(split.protocol_fee, 200);
        assert_eq!(split.lp_fee, 800);
        assert_eq!(split.referral_fee, 0);
        let with_referral = split_fees(&fees, 1_000, true);
        assert_eq!(with_referral.referral_fee, 40);
        assert_eq!(with_referral.protocol_fee, 160);
    }
}

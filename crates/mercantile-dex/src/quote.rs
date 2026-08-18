//! Swap quoting: what a trade would actually do to the pool, and cost.
//!
//! Ported from the `cp_amm` program's swap path for concentrated-liquidity pools,
//! so a quote and the transaction it precedes agree exactly (same rounding, same
//! fee handling, same range checks). Two modes are supported, matching the two the
//! Mercantile CLI uses:
//!
//! * **exact in** — spend a known amount (sell N items, or spend N GP)
//! * **exact out** — receive a known amount (buy exactly N items)
//!
//! Buys default to exact-out because items are only meaningful in whole units.

use mercantile_core::{base_to_gp, base_to_items};

use crate::fees::{exclude_fee, include_fee, split_fees, total_fee_numerator};
use crate::math::{
    amount_a_from_liquidity, amount_b_from_liquidity, next_sqrt_price_from_amount_a_in,
    next_sqrt_price_from_amount_a_out, next_sqrt_price_from_amount_b_in,
    next_sqrt_price_from_amount_b_out, Rounding,
};
use crate::pool::{FeeMode, PoolState, TradeDirection};
use crate::{DexError, Result};

/// Which side of the *item* the caller is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Spend GP, receive items.
    Buy,
    /// Spend items, receive GP.
    Sell,
}

impl Side {
    pub fn direction(self) -> TradeDirection {
        match self {
            Side::Buy => TradeDirection::BtoA,
            Side::Sell => TradeDirection::AtoB,
        }
    }

    /// The opposite side.
    pub fn flip(self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

/// The result of quoting a swap, in base units unless a field says otherwise.
#[derive(Debug, Clone, Copy)]
pub struct Quote {
    pub side: Side,
    /// Amount paid, including any fee charged on the way in.
    pub amount_in: u64,
    /// Amount received, after any fee charged on the way out.
    pub amount_out: u64,
    /// Pool sqrt price after the swap.
    pub next_sqrt_price: u128,
    /// Total trading fee (LP + protocol + referral), in whichever token it was taken.
    pub trade_fee: u128,
    /// Protocol's share of `trade_fee`.
    pub protocol_fee: u128,
    /// GP per whole item before the swap.
    pub spot_price: f64,
    /// GP per whole item actually paid or received across the whole fill.
    pub execution_price: f64,
    /// How far `execution_price` sits from `spot_price`, in percent.
    pub price_impact_pct: f64,
}

impl Quote {
    /// GP moved by this swap, in whole GP.
    pub fn gp(&self) -> f64 {
        match self.side {
            Side::Buy => base_to_gp(self.amount_in),
            Side::Sell => base_to_gp(self.amount_out),
        }
    }

    /// Items moved by this swap, in whole items.
    pub fn items(&self) -> f64 {
        match self.side {
            Side::Buy => base_to_items(self.amount_out),
            Side::Sell => base_to_items(self.amount_in),
        }
    }

    /// The worst amount-in the caller will accept for a `slippage_pct` tolerance.
    pub fn max_amount_in(&self, slippage_pct: f64) -> u64 {
        let scaled = (self.amount_in as f64 * (1.0 + slippage_pct / 100.0)).ceil();
        scaled.min(u64::MAX as f64) as u64
    }

    /// The worst amount-out the caller will accept for a `slippage_pct` tolerance.
    pub fn min_amount_out(&self, slippage_pct: f64) -> u64 {
        (self.amount_out as f64 * (1.0 - slippage_pct / 100.0))
            .floor()
            .max(0.0) as u64
    }
}

/// Quote a swap against a pool snapshot.
///
/// `current_point` is the pool's clock: a unix timestamp for `activation_type == 1`
/// (all Mercantile pools) or a slot otherwise. It only affects fee schedules.
pub trait SwapQuote {
    /// Spend exactly `amount_in` base units of the input token.
    fn quote_exact_in(&self, side: Side, amount_in: u64, current_point: u64) -> Result<Quote>;

    /// Receive exactly `amount_out` base units of the output token.
    fn quote_exact_out(&self, side: Side, amount_out: u64, current_point: u64) -> Result<Quote>;

    /// Buy exactly `items` whole items (exact-out), the natural way to size a buy.
    fn quote_buy_items(&self, items: u64, current_point: u64) -> Result<Quote> {
        self.quote_exact_out(
            Side::Buy,
            mercantile_core::items_to_base(items),
            current_point,
        )
    }

    /// Sell exactly `items` whole items (exact-in).
    fn quote_sell_items(&self, items: u64, current_point: u64) -> Result<Quote> {
        self.quote_exact_in(
            Side::Sell,
            mercantile_core::items_to_base(items),
            current_point,
        )
    }
}

impl SwapQuote for PoolState {
    fn quote_exact_in(&self, side: Side, amount_in: u64, current_point: u64) -> Result<Quote> {
        self.check_tradeable(amount_in, current_point)?;
        let direction = side.direction();
        let fee_mode = FeeMode::resolve(self.collect_fee_mode, direction, false);
        let fee_numerator = total_fee_numerator(
            &self.fees,
            self.fee_version,
            current_point,
            self.activation_point,
        )?;

        let mut trade_fee = 0u128;
        let mut protocol_fee = 0u128;

        // Fee on the way in (GP in, OnlyB pools) comes off before the curve is walked.
        let swap_amount_in = if fee_mode.fees_on_input {
            let (net, fee) = exclude_fee(fee_numerator, amount_in as u128);
            trade_fee = fee;
            protocol_fee = split_fees(&self.fees, fee, false).protocol_fee;
            net
        } else {
            amount_in as u128
        };

        let (gross_out, next_sqrt_price) = match direction {
            TradeDirection::AtoB => {
                let next = next_sqrt_price_from_amount_a_in(
                    self.sqrt_price,
                    self.liquidity,
                    swap_amount_in,
                )?;
                if next < self.sqrt_min_price {
                    return Err(self.range_error());
                }
                (
                    amount_b_from_liquidity(next, self.sqrt_price, self.liquidity, Rounding::Down)?,
                    next,
                )
            }
            TradeDirection::BtoA => {
                let next = next_sqrt_price_from_amount_b_in(
                    self.sqrt_price,
                    self.liquidity,
                    swap_amount_in,
                )?;
                if next > self.sqrt_max_price {
                    return Err(self.range_error());
                }
                (
                    amount_a_from_liquidity(self.sqrt_price, next, self.liquidity, Rounding::Down)?,
                    next,
                )
            }
        };

        // Otherwise the fee comes out of the proceeds.
        let amount_out = if fee_mode.fees_on_input {
            gross_out
        } else {
            let (net, fee) = exclude_fee(fee_numerator, gross_out);
            trade_fee = fee;
            protocol_fee = split_fees(&self.fees, fee, false).protocol_fee;
            net
        };

        self.finish(
            side,
            amount_in as u128,
            amount_out,
            next_sqrt_price,
            trade_fee,
            protocol_fee,
        )
    }

    fn quote_exact_out(&self, side: Side, amount_out: u64, current_point: u64) -> Result<Quote> {
        self.check_tradeable(amount_out, current_point)?;
        let direction = side.direction();
        let fee_mode = FeeMode::resolve(self.collect_fee_mode, direction, false);
        let fee_numerator = total_fee_numerator(
            &self.fees,
            self.fee_version,
            current_point,
            self.activation_point,
        )?;

        let mut trade_fee = 0u128;
        let mut protocol_fee = 0u128;

        // When the fee is taken on output, the curve must yield the requested
        // amount *plus* the fee, so gross the target up first.
        let gross_out = if fee_mode.fees_on_input {
            amount_out as u128
        } else {
            let (included, fee) = include_fee(fee_numerator, amount_out as u128)?;
            trade_fee = fee;
            protocol_fee = split_fees(&self.fees, fee, false).protocol_fee;
            included
        };

        let (swap_amount_in, next_sqrt_price) = match direction {
            TradeDirection::AtoB => {
                let next =
                    next_sqrt_price_from_amount_b_out(self.sqrt_price, self.liquidity, gross_out)?;
                if next < self.sqrt_min_price {
                    return Err(self.range_error());
                }
                (
                    amount_a_from_liquidity(next, self.sqrt_price, self.liquidity, Rounding::Up)?,
                    next,
                )
            }
            TradeDirection::BtoA => {
                let next =
                    next_sqrt_price_from_amount_a_out(self.sqrt_price, self.liquidity, gross_out)?;
                if next > self.sqrt_max_price {
                    return Err(self.range_error());
                }
                (
                    amount_b_from_liquidity(self.sqrt_price, next, self.liquidity, Rounding::Up)?,
                    next,
                )
            }
        };

        // A fee charged on input is added on top of what the curve consumed.
        let amount_in = if fee_mode.fees_on_input {
            let (included, fee) = include_fee(fee_numerator, swap_amount_in)?;
            trade_fee = fee;
            protocol_fee = split_fees(&self.fees, fee, false).protocol_fee;
            included
        } else {
            swap_amount_in
        };

        self.finish(
            side,
            amount_in,
            amount_out as u128,
            next_sqrt_price,
            trade_fee,
            protocol_fee,
        )
    }
}

impl PoolState {
    fn check_tradeable(&self, amount: u64, current_point: u64) -> Result<()> {
        if amount == 0 {
            return Err(DexError::ZeroAmount);
        }
        if self.pool_status != 0 {
            return Err(DexError::PoolDisabled);
        }
        if current_point < self.activation_point {
            return Err(DexError::NotActivated {
                activation_point: self.activation_point,
                now: current_point,
            });
        }
        if self.liquidity == 0 {
            return Err(DexError::NoLiquidity);
        }
        Ok(())
    }

    fn range_error(&self) -> DexError {
        DexError::PriceRangeViolation {
            min: self.sqrt_min_price,
            max: self.sqrt_max_price,
        }
    }

    fn finish(
        &self,
        side: Side,
        amount_in: u128,
        amount_out: u128,
        next_sqrt_price: u128,
        trade_fee: u128,
        protocol_fee: u128,
    ) -> Result<Quote> {
        let amount_in = u64::try_from(amount_in).map_err(|_| DexError::Overflow("amount_in"))?;
        let amount_out = u64::try_from(amount_out).map_err(|_| DexError::Overflow("amount_out"))?;
        if amount_out == 0 {
            // The curve rounded the fill to nothing — too small to be worth sending.
            return Err(DexError::ZeroAmount);
        }
        let (gp, items) = match side {
            Side::Buy => (base_to_gp(amount_in), base_to_items(amount_out)),
            Side::Sell => (base_to_gp(amount_out), base_to_items(amount_in)),
        };
        let spot_price = self.spot_price();
        let execution_price = if items > 0.0 { gp / items } else { f64::NAN };
        let price_impact_pct = if spot_price > 0.0 {
            (execution_price - spot_price).abs() / spot_price * 100.0
        } else {
            f64::NAN
        };
        Ok(Quote {
            side,
            amount_in,
            amount_out,
            next_sqrt_price,
            trade_fee,
            protocol_fee,
            spot_price,
            execution_price,
            price_impact_pct,
        })
    }
}

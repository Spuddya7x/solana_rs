//! Building `cp_amm` swap instructions.
//!
//! Anchor instruction data is `sha256("global:<name>")[..8]` followed by the
//! Borsh-encoded arguments. `swap2` takes `(amount_0, amount_1, swap_mode)`, whose
//! meaning depends on the mode — see [`SwapMode`].

use mercantile_core::{CP_AMM_POOL_AUTHORITY, CP_AMM_PROGRAM_ID};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

use crate::pool::PoolState;
use crate::quote::Side;

/// `swap2` discriminator, from the `cp_amm` IDL.
pub const SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

/// Seed of the Anchor event-authority PDA every `cp_amm` instruction carries.
pub const EVENT_AUTHORITY_SEED: &[u8] = b"__event_authority";

/// How `amount_0` / `amount_1` are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapMode {
    /// Spend `amount_0` exactly; fail below `amount_1` out.
    ExactIn = 0,
    /// Spend up to `amount_0`, filling what the range allows.
    PartialFill = 1,
    /// Receive `amount_0` exactly; spend at most `amount_1`.
    ExactOut = 2,
}

/// Everything the instruction builder needs that is not on the pool account.
#[derive(Debug, Clone)]
pub struct SwapParams {
    pub pool: Pubkey,
    pub payer: Pubkey,
    /// Payer's token account for the token being spent.
    pub input_token_account: Pubkey,
    /// Payer's token account for the token being received.
    pub output_token_account: Pubkey,
    pub mode: SwapMode,
    /// Exact-in: amount in. Exact-out: amount out.
    pub amount_0: u64,
    /// Exact-in: minimum amount out. Exact-out: maximum amount in.
    pub amount_1: u64,
}

/// The `cp_amm` event authority PDA.
pub fn event_authority() -> Pubkey {
    Pubkey::find_program_address(&[EVENT_AUTHORITY_SEED], &CP_AMM_PROGRAM_ID).0
}

/// Build a `swap2` instruction against `pool`.
///
/// Account order is fixed by the IDL. `referral_token_account` is an optional
/// Anchor account, which is encoded by passing the program id in its slot.
pub fn swap_instruction(
    pool_state: &PoolState,
    params: &SwapParams,
    token_a_program: Pubkey,
    token_b_program: Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(8 + 17);
    data.extend_from_slice(&SWAP2_DISCRIMINATOR);
    data.extend_from_slice(&params.amount_0.to_le_bytes());
    data.extend_from_slice(&params.amount_1.to_le_bytes());
    data.push(params.mode as u8);

    Instruction {
        program_id: CP_AMM_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(CP_AMM_POOL_AUTHORITY, false),
            AccountMeta::new(params.pool, false),
            AccountMeta::new(params.input_token_account, false),
            AccountMeta::new(params.output_token_account, false),
            AccountMeta::new(pool_state.token_a_vault, false),
            AccountMeta::new(pool_state.token_b_vault, false),
            AccountMeta::new_readonly(pool_state.token_a_mint, false),
            AccountMeta::new_readonly(pool_state.token_b_mint, false),
            AccountMeta::new(params.payer, true),
            AccountMeta::new_readonly(token_a_program, false),
            AccountMeta::new_readonly(token_b_program, false),
            // Optional account, unused: Anchor reads the program id as "None".
            AccountMeta::new_readonly(CP_AMM_PROGRAM_ID, false),
            AccountMeta::new_readonly(event_authority(), false),
            AccountMeta::new_readonly(CP_AMM_PROGRAM_ID, false),
        ],
        data,
    }
}

/// Which of the payer's token accounts is the input for a given side.
pub fn token_accounts_for(
    side: Side,
    item_token_account: Pubkey,
    gp_token_account: Pubkey,
) -> (Pubkey, Pubkey) {
    match side {
        Side::Buy => (gp_token_account, item_token_account),
        Side::Sell => (item_token_account, gp_token_account),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_authority_is_the_anchor_pda() {
        // Stable across every cp-amm instruction; pinned so a refactor cannot drift.
        assert_eq!(
            event_authority().to_string(),
            "3rmHSu74h1ZcmAisVcWerTCiRDQbUrBKmcwptYGjHfet"
        );
    }

    #[test]
    fn swap_data_is_discriminator_then_args() {
        let data = {
            let mut d = Vec::new();
            d.extend_from_slice(&SWAP2_DISCRIMINATOR);
            d.extend_from_slice(&1_234u64.to_le_bytes());
            d.extend_from_slice(&5_678u64.to_le_bytes());
            d.push(SwapMode::ExactOut as u8);
            d
        };
        assert_eq!(data.len(), 25);
        assert_eq!(&data[..8], &SWAP2_DISCRIMINATOR);
        assert_eq!(data[24], 2);
    }

    #[test]
    fn buy_spends_gp_and_sell_spends_items() {
        let item = Pubkey::new_unique();
        let gp = Pubkey::new_unique();
        assert_eq!(token_accounts_for(Side::Buy, item, gp), (gp, item));
        assert_eq!(token_accounts_for(Side::Sell, item, gp), (item, gp));
    }
}

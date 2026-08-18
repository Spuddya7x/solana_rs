//! Thin RPC layer: read pool state and balances, and send swaps.
//!
//! Deliberately blocking. The bot is a poll-decide-act loop against a handful of
//! accounts per tick, so an async runtime would add machinery without buying
//! throughput, and a blocking client keeps strategy code trivial to test.

use std::time::Duration;

use mercantile_core::{GP_MINT, ITEM_DECIMALS};
use solana_account_decoder_client_types::UiAccountData;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature};
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;

use crate::pool::PoolState;
use crate::quote::{Quote, Side};
use crate::swap::{swap_instruction, token_accounts_for, SwapMode, SwapParams};
use crate::{DexError, Result};

/// Item and GP mints are both classic SPL Token (not Token-2022) after the
/// migration described in `chain/cli/swap.ts`.
pub const TOKEN_PROGRAM_ID: Pubkey = spl_token::ID;

/// Reading pool state — the only thing strategies need from the chain.
///
/// Kept as a trait so tests and backtests can feed synthetic pools to the same
/// code path the live bot uses.
pub trait PoolFetch {
    /// Fetch several pools at once. Missing or undecodable accounts come back `None`.
    fn fetch_pools(&self, pools: &[Pubkey]) -> Result<Vec<Option<PoolState>>>;

    /// The pool clock: unix timestamp for timestamp-activated pools.
    fn current_point(&self) -> Result<u64>;
}

/// An RPC-backed chain client.
pub struct ChainClient {
    rpc: RpcClient,
    /// Extra micro-lamports per compute unit on swap transactions.
    priority_fee_micro_lamports: u64,
    compute_unit_limit: u32,
}

impl ChainClient {
    /// Connect with the given commitment.
    pub fn new(url: impl Into<String>, commitment: CommitmentConfig) -> Self {
        Self {
            rpc: RpcClient::new_with_timeout_and_commitment(
                url.into(),
                Duration::from_secs(30),
                commitment,
            ),
            priority_fee_micro_lamports: 0,
            compute_unit_limit: 400_000,
        }
    }

    /// Set the priority fee applied to swap transactions.
    pub fn with_priority_fee(mut self, micro_lamports: u64) -> Self {
        self.priority_fee_micro_lamports = micro_lamports;
        self
    }

    /// Underlying RPC client, for callers that need something not wrapped here.
    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }

    /// Fetch and decode a single pool.
    pub fn fetch_pool(&self, pool: &Pubkey) -> Result<PoolState> {
        let account = self
            .rpc
            .get_account(pool)
            .map_err(|e| DexError::Rpc(Box::new(e)))?;
        PoolState::decode(&account.data)
    }

    /// SOL balance in lamports — the bot needs this to know it can still pay fees.
    pub fn sol_balance(&self, owner: &Pubkey) -> Result<u64> {
        self.rpc
            .get_balance(owner)
            .map_err(|e| DexError::Rpc(Box::new(e)))
    }

    /// Balance of an SPL token account in base units. A missing account reads as zero,
    /// which is what an owner who has never held the token effectively has.
    pub fn token_balance(&self, owner: &Pubkey, mint: &Pubkey) -> Result<u64> {
        let ata = get_associated_token_address_with_program_id(owner, mint, &TOKEN_PROGRAM_ID);
        match self.rpc.get_token_account_balance(&ata) {
            Ok(balance) => Ok(balance.amount.parse().unwrap_or(0)),
            Err(err) if is_missing_account(&err) => Ok(0),
            Err(err) => Err(DexError::Rpc(Box::new(err))),
        }
    }

    /// Total supply of each mint, in whole tokens.
    ///
    /// Reads the SPL mint accounts directly rather than calling `getTokenSupply`
    /// per mint: supply is a `u64` at offset 36 of the 82-byte mint layout and
    /// decimals a `u8` at offset 44, so a whole universe costs one batched call
    /// instead of one call per market.
    pub fn token_supplies(&self, mints: &[Pubkey]) -> Result<Vec<Option<f64>>> {
        const SUPPLY_OFFSET: usize = 36;
        const DECIMALS_OFFSET: usize = 44;
        const MINT_LEN: usize = 82;

        let mut out = Vec::with_capacity(mints.len());
        for chunk in mints.chunks(100) {
            let accounts = self
                .rpc
                .get_multiple_accounts(chunk)
                .map_err(|e| DexError::Rpc(Box::new(e)))?;
            for account in accounts {
                let supply = account.and_then(|account| {
                    if account.data.len() < MINT_LEN {
                        return None;
                    }
                    let raw = u64::from_le_bytes(
                        account.data[SUPPLY_OFFSET..SUPPLY_OFFSET + 8]
                            .try_into()
                            .ok()?,
                    );
                    let decimals = account.data[DECIMALS_OFFSET];
                    Some(raw as f64 / 10f64.powi(decimals as i32))
                });
                out.push(supply);
            }
        }
        Ok(out)
    }

    /// GP balance in base units.
    pub fn gp_balance(&self, owner: &Pubkey) -> Result<u64> {
        self.token_balance(owner, &GP_MINT)
    }

    /// Every item token the owner holds, as `(mint, base units)`, zero balances dropped.
    pub fn item_balances(&self, owner: &Pubkey) -> Result<Vec<(Pubkey, u64)>> {
        use solana_client::rpc_request::TokenAccountsFilter;
        let accounts = self
            .rpc
            .get_token_accounts_by_owner(owner, TokenAccountsFilter::ProgramId(TOKEN_PROGRAM_ID))
            .map_err(|e| DexError::Rpc(Box::new(e)))?;
        let mut out = Vec::new();
        for keyed in accounts {
            let solana_client::rpc_response::RpcKeyedAccount { account, .. } = keyed;
            if let UiAccountData::Json(parsed) = account.data {
                let info = &parsed.parsed["info"];
                let mint = info["mint"].as_str().and_then(|m| m.parse::<Pubkey>().ok());
                let amount = info["tokenAmount"]["amount"]
                    .as_str()
                    .and_then(|a| a.parse::<u64>().ok());
                let (Some(mint), Some(amount)) = (mint, amount) else {
                    continue;
                };
                if amount > 0 && mint != GP_MINT {
                    out.push((mint, amount));
                }
            }
        }
        Ok(out)
    }

    /// Execute a swap: ensure the output token account exists, then swap.
    ///
    /// `quote` fixes the direction and size; `slippage_pct` sets the guard rail the
    /// program will enforce (`minimum_amount_out` or `maximum_amount_in`).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_swap(
        &self,
        pool_address: &Pubkey,
        pool: &PoolState,
        item_mint: &Pubkey,
        payer: &Keypair,
        quote: &Quote,
        slippage_pct: f64,
        exact_out: bool,
    ) -> Result<Signature> {
        let owner = payer.pubkey();
        let item_ata =
            get_associated_token_address_with_program_id(&owner, item_mint, &TOKEN_PROGRAM_ID);
        let gp_ata =
            get_associated_token_address_with_program_id(&owner, &GP_MINT, &TOKEN_PROGRAM_ID);
        let (input_token_account, output_token_account) =
            token_accounts_for(quote.side, item_ata, gp_ata);
        let output_mint = match quote.side {
            Side::Buy => *item_mint,
            Side::Sell => GP_MINT,
        };

        let (mode, amount_0, amount_1) = if exact_out {
            (
                SwapMode::ExactOut,
                quote.amount_out,
                quote.max_amount_in(slippage_pct),
            )
        } else {
            (
                SwapMode::ExactIn,
                quote.amount_in,
                quote.min_amount_out(slippage_pct),
            )
        };

        let mut instructions: Vec<Instruction> = Vec::new();
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(
            self.compute_unit_limit,
        ));
        if self.priority_fee_micro_lamports > 0 {
            instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
                self.priority_fee_micro_lamports,
            ));
        }
        // Idempotent: costs nothing if the account is already there, and saves the
        // swap from failing on a token the wallet has never held.
        instructions.push(create_associated_token_account_idempotent(
            &owner,
            &owner,
            &output_mint,
            &TOKEN_PROGRAM_ID,
        ));
        instructions.push(swap_instruction(
            pool,
            &SwapParams {
                pool: *pool_address,
                payer: owner,
                input_token_account,
                output_token_account,
                mode,
                amount_0,
                amount_1,
            },
            TOKEN_PROGRAM_ID,
            TOKEN_PROGRAM_ID,
        ));

        let blockhash = self
            .rpc
            .get_latest_blockhash()
            .map_err(|e| DexError::Rpc(Box::new(e)))?;
        let tx =
            Transaction::new_signed_with_payer(&instructions, Some(&owner), &[payer], blockhash);
        self.rpc
            .send_and_confirm_transaction(&tx)
            .map_err(|e| DexError::Rpc(Box::new(e)))
    }
}

impl PoolFetch for ChainClient {
    fn fetch_pools(&self, pools: &[Pubkey]) -> Result<Vec<Option<PoolState>>> {
        let mut out = Vec::with_capacity(pools.len());
        // getMultipleAccounts caps at 100 keys per request.
        for chunk in pools.chunks(100) {
            let accounts = self
                .rpc
                .get_multiple_accounts(chunk)
                .map_err(|e| DexError::Rpc(Box::new(e)))?;
            for (account, address) in accounts.into_iter().zip(chunk) {
                match account {
                    Some(account) => match PoolState::decode(&account.data) {
                        Ok(state) => out.push(Some(state)),
                        Err(err) => {
                            // A pool we cannot decode is one we must not trade.
                            tracing::warn!(pool = %address, %err, "skipping undecodable pool");
                            out.push(None);
                        }
                    },
                    None => out.push(None),
                }
            }
        }
        Ok(out)
    }

    fn current_point(&self) -> Result<u64> {
        let slot = self
            .rpc
            .get_slot()
            .map_err(|e| DexError::Rpc(Box::new(e)))?;
        let block_time = self
            .rpc
            .get_block_time(slot)
            .map_err(|e| DexError::Rpc(Box::new(e)))?;
        Ok(block_time.max(0) as u64)
    }
}

/// Item token accounts are 1-decimal; used when reporting balances.
pub const fn item_decimals() -> u8 {
    ITEM_DECIMALS
}

fn is_missing_account(err: &solana_client::client_error::ClientError) -> bool {
    // The RPC reports a never-created ATA as an invalid param rather than a null
    // account, so match on the message rather than the error kind.
    let text = err.to_string();
    text.contains("could not find account") || text.contains("Invalid param")
}

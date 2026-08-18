//! Turning approved orders into fills — simulated or real.
//!
//! Both executors quote against the *same* live pool state the strategies saw, so
//! a paper run and a live run differ only in whether a transaction is sent. That
//! is the point: paper results are meaningful precisely because nothing else changes.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{read_keypair_file, Keypair};
use solana_sdk::signer::Signer;

use mercantile_dex::client::ChainClient;
use mercantile_dex::quote::{Quote, Side};

use crate::market::MarketView;
use crate::risk::Order;

/// A completed trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    /// Unix seconds.
    pub ts: i64,
    /// Registry key of the market.
    pub market: String,
    pub side: Side,
    /// Whole items traded.
    pub items: u64,
    /// GP spent (buy) or received (sell).
    pub gp: f64,
    /// GP per item actually paid or received.
    pub price: f64,
    /// Pool fee paid, in GP.
    pub fee_gp: f64,
    /// Transaction signature, absent in paper mode.
    pub signature: Option<String>,
    pub paper: bool,
    /// Which strategy asked for this.
    pub strategy: String,
}

/// How orders are turned into fills.
pub trait Executor {
    /// Human-readable mode, for logs and journals.
    fn mode(&self) -> &'static str;

    /// Execute an approved order, or explain why it did not happen.
    fn execute(&mut self, order: &Order, view: &MarketView<'_>) -> Result<Fill>;
}

/// Simulated execution against real quotes.
///
/// Fills are priced from a fresh quote on the current pool state and then made
/// slightly worse by `slippage_bps`, standing in for the delay between deciding
/// and landing. Nothing is sent and no keypair is needed.
pub struct PaperExecutor {
    slippage_bps: f64,
}

impl PaperExecutor {
    pub fn new(slippage_bps: f64) -> Self {
        Self { slippage_bps }
    }
}

impl Executor for PaperExecutor {
    fn mode(&self) -> &'static str {
        "paper"
    }

    fn execute(&mut self, order: &Order, view: &MarketView<'_>) -> Result<Fill> {
        let quote = requote(order, view)?;
        let penalty = 1.0 + self.slippage_bps / 10_000.0;
        let (gp, items) = (quote.gp(), quote.items());
        // Paper pays a little more when buying and receives a little less when selling.
        let gp = match order.side {
            Side::Buy => gp * penalty,
            Side::Sell => gp / penalty,
        };
        let price = if items > 0.0 { gp / items } else { 0.0 };
        check_limit(order, price)?;
        Ok(Fill {
            ts: view.now,
            market: order.market.clone(),
            side: order.side,
            items: order.items,
            gp,
            price,
            fee_gp: mercantile_core::base_to_gp(quote.trade_fee.min(u64::MAX as u128) as u64),
            signature: None,
            paper: true,
            strategy: order.strategy.clone(),
        })
    }
}

/// Real execution: builds, signs, sends and confirms a swap.
///
/// The fill is measured from the wallet's actual balance change rather than the
/// quote, so the journal records what happened rather than what was expected.
pub struct LiveExecutor {
    client: ChainClient,
    keypair: Keypair,
    slippage_pct: f64,
}

impl LiveExecutor {
    /// Load the signing keypair and prepare to trade.
    pub fn new(client: ChainClient, keypair_path: &Path, slippage_pct: f64) -> Result<Self> {
        let keypair = read_keypair_file(keypair_path)
            .map_err(|e| anyhow::anyhow!("reading keypair {}: {e}", keypair_path.display()))?;
        Ok(Self {
            client,
            keypair,
            slippage_pct,
        })
    }

    /// The wallet that will sign.
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    pub fn client(&self) -> &ChainClient {
        &self.client
    }
}

impl Executor for LiveExecutor {
    fn mode(&self) -> &'static str {
        "live"
    }

    fn execute(&mut self, order: &Order, view: &MarketView<'_>) -> Result<Fill> {
        let quote = requote(order, view)?;
        check_limit(order, quote.execution_price)?;

        let owner = self.keypair.pubkey();
        let received_mint = match order.side {
            Side::Buy => view.mint(),
            Side::Sell => mercantile_core::GP_MINT,
        };
        let before = self
            .client
            .token_balance(&owner, &received_mint)
            .context("reading balance before the swap")?;

        // Buys are exact-out so the item count is exact; sells are exact-in so the
        // item count leaving the wallet is exact. Either way the size is honoured
        // and the price is the thing that floats within the slippage guard.
        let exact_out = order.side == Side::Buy;
        let signature = self
            .client
            .execute_swap(
                &view.pool_address(),
                view.pool,
                &view.mint(),
                &self.keypair,
                &quote,
                self.slippage_pct,
                exact_out,
            )
            .context("sending the swap")?;

        let after = self
            .client
            .token_balance(&owner, &received_mint)
            .context("reading balance after the swap")?;
        let received = after.saturating_sub(before);

        let (gp, items) = match order.side {
            Side::Buy => (quote.gp(), mercantile_core::base_to_items(received)),
            Side::Sell => (mercantile_core::base_to_gp(received), quote.items()),
        };
        let price = if items > 0.0 { gp / items } else { 0.0 };
        Ok(Fill {
            ts: view.now,
            market: order.market.clone(),
            side: order.side,
            items: items.round() as u64,
            gp,
            price,
            fee_gp: mercantile_core::base_to_gp(quote.trade_fee.min(u64::MAX as u128) as u64),
            signature: Some(signature.to_string()),
            paper: false,
            strategy: order.strategy.clone(),
        })
    }
}

/// Re-quote at execution time: the pool may have moved since the strategy looked.
fn requote(order: &Order, view: &MarketView<'_>) -> Result<Quote> {
    let quote = match order.side {
        Side::Buy => view.quote_buy(order.items),
        Side::Sell => view.quote_sell(order.items),
    };
    quote.ok_or_else(|| {
        anyhow::anyhow!(
            "{} {} x{} no longer quotes against the pool",
            order.side.as_str(),
            order.market,
            order.items
        )
    })
}

/// Refuse a fill that has drifted past the order's limit price.
fn check_limit(order: &Order, price: f64) -> Result<()> {
    if let Some(limit) = order.limit_price {
        let ok = match order.side {
            Side::Buy => price <= limit,
            Side::Sell => price >= limit,
        };
        if !ok {
            anyhow::bail!(
                "{} {} at {price:.4} breaches the {limit:.4} limit",
                order.side.as_str(),
                order.market
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::Order;
    use crate::testing::TestMarket;

    fn order(side: Side, items: u64, limit: Option<f64>) -> Order {
        Order {
            market: "lobster".to_string(),
            side,
            items,
            limit_price: limit,
            strategy: "test".to_string(),
            reason: "test".to_string(),
        }
    }

    #[test]
    fn paper_fills_cost_more_than_the_raw_quote() {
        let market = TestMarket::above_floor(1.2);
        let view = market.view();
        let raw = view.quote_buy(5).unwrap();
        let mut executor = PaperExecutor::new(50.0); // 0.5%
        let fill = executor.execute(&order(Side::Buy, 5, None), &view).unwrap();
        assert!(fill.paper);
        assert!(fill.signature.is_none());
        assert_eq!(fill.items, 5);
        assert!(fill.gp > raw.gp(), "{} vs {}", fill.gp, raw.gp());
        assert!(fill.gp < raw.gp() * 1.01);
    }

    #[test]
    fn paper_sells_receive_less_than_the_raw_quote() {
        let market = TestMarket::above_floor(1.6);
        let view = market.view();
        let raw = view.quote_sell(2).unwrap();
        let mut executor = PaperExecutor::new(50.0);
        let fill = executor
            .execute(&order(Side::Sell, 2, None), &view)
            .unwrap();
        assert!(fill.gp < raw.gp());
    }

    #[test]
    fn a_breached_limit_stops_the_fill() {
        let market = TestMarket::above_floor(1.2);
        let view = market.view();
        let mut executor = PaperExecutor::new(0.0);
        let err = executor
            .execute(&order(Side::Buy, 5, Some(1.0)), &view)
            .unwrap_err();
        assert!(err.to_string().contains("limit"), "{err}");
    }

    #[test]
    fn an_unquotable_order_fails_loudly() {
        let market = TestMarket::at_floor();
        let view = market.view();
        let mut executor = PaperExecutor::new(0.0);
        let err = executor
            .execute(&order(Side::Sell, 5, None), &view)
            .unwrap_err();
        assert!(err.to_string().contains("no longer quotes"), "{err}");
    }
}

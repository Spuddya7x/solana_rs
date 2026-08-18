//! Positions, cash and P&L.
//!
//! Quantities are authoritative from the chain in live mode; cost basis is only
//! knowable from the bot's own fills, so the two are tracked separately and
//! reconciled by [`Portfolio::sync_balances`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::execution::Fill;
use mercantile_dex::quote::Side;

/// A holding in one market.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Whole items held.
    pub items: u64,
    /// Weighted average acquisition price, in GP per item.
    pub avg_cost_gp: f64,
}

impl Position {
    /// GP originally paid for what is still held.
    pub fn cost_basis(&self) -> f64 {
        self.items as f64 * self.avg_cost_gp
    }

    /// Mark-to-market value at `price`.
    pub fn market_value(&self, price: f64) -> f64 {
        self.items as f64 * price
    }

    /// Unrealised P&L at `price`.
    pub fn unrealized(&self, price: f64) -> f64 {
        self.market_value(price) - self.cost_basis()
    }
}

/// The bot's book.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Portfolio {
    /// Spendable GP.
    pub gp: f64,
    /// Positions keyed by registry key.
    pub positions: BTreeMap<String, Position>,
    /// Realised P&L for the session, net of fees paid on the way out.
    pub realized_pnl: f64,
    /// Total GP paid in pool fees.
    pub fees_paid: f64,
    /// Fills applied.
    pub trade_count: u64,
}

impl Portfolio {
    /// Open a book with a starting GP balance.
    pub fn new(gp: f64) -> Self {
        Self {
            gp,
            ..Default::default()
        }
    }

    pub fn position(&self, key: &str) -> Position {
        self.positions.get(key).copied().unwrap_or_default()
    }

    /// Apply a fill, moving cash and updating cost basis.
    ///
    /// A sell realises P&L against the average cost of what is being sold; selling
    /// more than the book knows about (possible in live mode if the wallet was
    /// funded elsewhere) is treated as zero-cost inventory rather than a negative
    /// position.
    pub fn apply_fill(&mut self, fill: &Fill) {
        self.trade_count += 1;
        self.fees_paid += fill.fee_gp;
        let entry = self.positions.entry(fill.market.clone()).or_default();
        match fill.side {
            Side::Buy => {
                let new_items = entry.items + fill.items;
                if new_items > 0 {
                    entry.avg_cost_gp = (entry.cost_basis() + fill.gp) / new_items as f64;
                }
                entry.items = new_items;
                self.gp -= fill.gp;
            }
            Side::Sell => {
                let sold = fill.items.min(entry.items);
                let untracked = fill.items - sold;
                self.realized_pnl += fill.gp - sold as f64 * entry.avg_cost_gp;
                entry.items -= sold;
                if entry.items == 0 {
                    entry.avg_cost_gp = 0.0;
                }
                if untracked > 0 {
                    tracing::warn!(
                        market = %fill.market,
                        untracked,
                        "sold more items than the book held; treating the excess as zero-cost"
                    );
                }
                self.gp += fill.gp;
            }
        }
        if self.positions[&fill.market].items == 0 {
            self.positions.remove(&fill.market);
        }
    }

    /// Replace quantities with on-chain truth, keeping cost basis for anything
    /// still held. Positions that appear from nowhere (a bridge withdrawal, say)
    /// enter the book at zero cost so they cannot fake a profit.
    pub fn sync_balances(&mut self, gp: f64, items: &BTreeMap<String, u64>) {
        self.gp = gp;
        for (key, count) in items {
            let entry = self.positions.entry(key.clone()).or_default();
            entry.items = *count;
        }
        self.positions.retain(|key, position| {
            let held = items.get(key).copied().unwrap_or(0);
            position.items = held;
            held > 0
        });
    }

    /// Total cost basis across every market.
    pub fn total_exposure(&self) -> f64 {
        self.positions.values().map(Position::cost_basis).sum()
    }

    /// GP plus mark-to-market value of every position.
    pub fn equity(&self, prices: &BTreeMap<String, f64>) -> f64 {
        self.gp
            + self
                .positions
                .iter()
                .map(|(key, position)| {
                    position.market_value(prices.get(key).copied().unwrap_or(position.avg_cost_gp))
                })
                .sum::<f64>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(market: &str, side: Side, items: u64, gp: f64) -> Fill {
        Fill {
            ts: 0,
            market: market.to_string(),
            side,
            items,
            gp,
            price: gp / items as f64,
            fee_gp: 0.0,
            signature: None,
            paper: true,
            strategy: "test".to_string(),
        }
    }

    #[test]
    fn buying_averages_the_cost_basis() {
        let mut book = Portfolio::new(1_000.0);
        book.apply_fill(&fill("lobster", Side::Buy, 10, 500.0)); // 50/item
        book.apply_fill(&fill("lobster", Side::Buy, 10, 700.0)); // 70/item
        let position = book.position("lobster");
        assert_eq!(position.items, 20);
        assert!((position.avg_cost_gp - 60.0).abs() < 1e-9);
        assert!((book.gp + 200.0).abs() < 1e-9, "spent 1200 of 1000");
    }

    #[test]
    fn selling_realises_against_average_cost() {
        let mut book = Portfolio::new(1_000.0);
        book.apply_fill(&fill("lobster", Side::Buy, 10, 500.0));
        book.apply_fill(&fill("lobster", Side::Sell, 4, 280.0)); // 70/item
        assert!((book.realized_pnl - 80.0).abs() < 1e-9, "4 x (70 - 50)");
        assert_eq!(book.position("lobster").items, 6);
        assert!((book.gp - 780.0).abs() < 1e-9);
    }

    #[test]
    fn closing_a_position_clears_it() {
        let mut book = Portfolio::new(1_000.0);
        book.apply_fill(&fill("lobster", Side::Buy, 5, 250.0));
        book.apply_fill(&fill("lobster", Side::Sell, 5, 300.0));
        assert_eq!(book.position("lobster"), Position::default());
        assert!(!book.positions.contains_key("lobster"));
        assert!((book.realized_pnl - 50.0).abs() < 1e-9);
    }

    #[test]
    fn selling_untracked_inventory_does_not_go_negative() {
        let mut book = Portfolio::new(0.0);
        book.apply_fill(&fill("shark", Side::Sell, 3, 300.0));
        assert_eq!(book.position("shark").items, 0);
        assert!((book.realized_pnl - 300.0).abs() < 1e-9);
        assert!((book.gp - 300.0).abs() < 1e-9);
    }

    #[test]
    fn syncing_balances_keeps_cost_basis_and_drops_empties() {
        let mut book = Portfolio::new(100.0);
        book.apply_fill(&fill("lobster", Side::Buy, 10, 500.0));
        book.apply_fill(&fill("shark", Side::Buy, 2, 400.0));

        let mut on_chain = BTreeMap::new();
        on_chain.insert("lobster".to_string(), 7); // three left the wallet elsewhere
        book.sync_balances(2_000.0, &on_chain);

        assert_eq!(book.gp, 2_000.0);
        assert_eq!(book.position("lobster").items, 7);
        assert!((book.position("lobster").avg_cost_gp - 50.0).abs() < 1e-9);
        assert!(!book.positions.contains_key("shark"), "sharks are gone");
    }

    #[test]
    fn equity_marks_positions_to_market() {
        let mut book = Portfolio::new(1_000.0);
        book.apply_fill(&fill("lobster", Side::Buy, 10, 500.0));
        let prices = BTreeMap::from([("lobster".to_string(), 65.0)]);
        assert!((book.equity(&prices) - (500.0 + 650.0)).abs() < 1e-9);
        assert!((book.position("lobster").unrealized(65.0) - 150.0).abs() < 1e-9);
    }
}

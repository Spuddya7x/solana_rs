//! The risk manager: the only place that can authorise a trade.
//!
//! Strategies express intent; this turns intent into an [`Order`] or a refusal.
//! Every limit is checked here rather than inside strategies, so adding a strategy
//! cannot widen the bot's risk, and so the reason a trade did not happen is always
//! recorded in one place.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use mercantile_dex::quote::Side;

use crate::config::RiskConfig;
use crate::execution::Fill;
use crate::market::MarketView;
use crate::portfolio::Portfolio;
use crate::strategy::Signal;

/// An approved, sized order.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub market: String,
    pub side: Side,
    /// Whole items, after any clamping.
    pub items: u64,
    /// Worst acceptable price in GP per item.
    pub limit_price: Option<f64>,
    pub strategy: String,
    pub reason: String,
}

/// What the risk manager decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Trade it, possibly smaller than asked.
    Approve(Order),
    /// Do not trade, with a reason worth journalling.
    Reject(RejectReason),
}

/// Why an order was refused, in a form that reads well in a log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectReason {
    pub market: String,
    pub strategy: String,
    pub rule: String,
    pub detail: String,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]: {}", self.market, self.rule, self.detail)
    }
}

/// Applies [`RiskConfig`] to every signal, and remembers enough between ticks to
/// enforce cooldowns, rate limits and a loss kill-switch.
pub struct RiskManager {
    config: RiskConfig,
    last_trade: HashMap<String, i64>,
    recent_trades: VecDeque<i64>,
    halted: Option<String>,
}

impl RiskManager {
    pub fn new(config: RiskConfig) -> Self {
        Self {
            config,
            last_trade: HashMap::new(),
            recent_trades: VecDeque::new(),
            halted: None,
        }
    }

    /// Why the bot has stopped trading, if it has.
    pub fn halted(&self) -> Option<&str> {
        self.halted.as_deref()
    }

    /// Trip the kill switch by hand (used by the engine on repeated failures).
    pub fn halt(&mut self, reason: impl Into<String>) {
        self.halted = Some(reason.into());
    }

    /// Judge one signal.
    pub fn evaluate(
        &mut self,
        strategy: &str,
        signal: &Signal,
        view: &MarketView<'_>,
        portfolio: &Portfolio,
        sol_balance: Option<f64>,
    ) -> Decision {
        let reject = |rule: &str, detail: String| {
            Decision::Reject(RejectReason {
                market: view.key().to_string(),
                strategy: strategy.to_string(),
                rule: rule.to_string(),
                detail,
            })
        };

        if let Some(reason) = &self.halted {
            return reject("halted", reason.clone());
        }
        if signal.items == 0 {
            return reject("empty", "signal asked for zero items".to_string());
        }

        // Loss kill switch, checked before anything else can add to the damage.
        if self.config.daily_loss_limit_gp > 0.0
            && portfolio.realized_pnl <= -self.config.daily_loss_limit_gp
        {
            let detail = format!(
                "realised {:.0} GP of losses, limit {:.0}",
                -portfolio.realized_pnl, self.config.daily_loss_limit_gp
            );
            self.halted = Some(detail.clone());
            return reject("daily_loss_limit", detail);
        }

        if let Some(sol) = sol_balance {
            if sol < self.config.min_sol_balance {
                let detail = format!(
                    "{sol:.4} SOL left, {:.4} required for fees",
                    self.config.min_sol_balance
                );
                return reject("min_sol_balance", detail);
            }
        }

        let now = view.now;
        if let Some(last) = self.last_trade.get(view.key()) {
            let elapsed = now - last;
            if elapsed < self.config.trade_cooldown_secs {
                return reject(
                    "cooldown",
                    format!(
                        "{elapsed}s since the last trade, {}s cooldown",
                        self.config.trade_cooldown_secs
                    ),
                );
            }
        }

        self.expire_trade_window(now);
        if self.recent_trades.len() >= self.config.max_trades_per_hour {
            return reject(
                "rate_limit",
                format!(
                    "{} trades in the last hour, limit {}",
                    self.recent_trades.len(),
                    self.config.max_trades_per_hour
                ),
            );
        }

        let position = view.position;
        let mut items = signal.items;

        match signal.side {
            Side::Buy => {
                let room = self
                    .config
                    .max_position_items
                    .saturating_sub(position.items);
                if room == 0 {
                    return reject(
                        "max_position_items",
                        format!("already holding {} items", position.items),
                    );
                }
                items = items.min(room);
            }
            Side::Sell => {
                if position.items == 0 {
                    return reject("no_position", "nothing to sell".to_string());
                }
                items = items.min(position.items);
            }
        }

        // Size against the money limits using a real quote, shrinking rather than
        // refusing where a smaller trade would still be worth doing.
        let mut quote = match self.quote(view, signal.side, items) {
            Some(quote) => quote,
            None => return reject("unquotable", format!("the pool cannot fill {items} items")),
        };

        if signal.side == Side::Buy {
            let spendable = (portfolio.gp - self.config.reserve_gp)
                .min(self.config.max_gp_per_trade)
                .min((self.config.max_position_gp_per_market - position.cost_basis()).max(0.0))
                .min((self.config.max_total_exposure_gp - portfolio.total_exposure()).max(0.0));
            if spendable <= 0.0 {
                return reject(
                    "no_budget",
                    format!(
                        "no GP available under the limits (balance {:.0}, exposure {:.0})",
                        portfolio.gp,
                        portfolio.total_exposure()
                    ),
                );
            }
            while quote.gp() > spendable && items > 1 {
                items -= 1;
                match self.quote(view, signal.side, items) {
                    Some(next) => quote = next,
                    None => {
                        return reject("unquotable", format!("the pool cannot fill {items} items"))
                    }
                }
            }
            if quote.gp() > spendable {
                return reject(
                    "max_gp_per_trade",
                    format!(
                        "even one item costs {:.0} GP, budget {spendable:.0}",
                        quote.gp()
                    ),
                );
            }
        } else if quote.gp() > self.config.max_gp_per_trade {
            while quote.gp() > self.config.max_gp_per_trade && items > 1 {
                items -= 1;
                match self.quote(view, signal.side, items) {
                    Some(next) => quote = next,
                    None => {
                        return reject("unquotable", format!("the pool cannot fill {items} items"))
                    }
                }
            }
        }

        if quote.gp() < self.config.min_gp_per_trade {
            return reject(
                "min_gp_per_trade",
                format!(
                    "{:.2} GP is below the {:.2} GP minimum",
                    quote.gp(),
                    self.config.min_gp_per_trade
                ),
            );
        }

        if quote.price_impact_pct > self.config.max_price_impact_pct {
            return reject(
                "max_price_impact",
                format!(
                    "{:.2}% impact exceeds the {:.2}% cap",
                    quote.price_impact_pct, self.config.max_price_impact_pct
                ),
            );
        }

        if !signal.accepts(quote.execution_price) {
            return reject(
                "limit_price",
                format!(
                    "{:.4} GP/item breaches the strategy's {:.4} limit",
                    quote.execution_price,
                    signal.limit_price.unwrap_or_default()
                ),
            );
        }

        // Optional: insist the pool could buy the position back.
        if signal.side == Side::Buy && self.config.min_exit_depth_mult > 0.0 {
            let depth = view.exit_depth_gp();
            let required = quote.gp() * self.config.min_exit_depth_mult;
            if depth < required {
                return reject(
                    "exit_depth",
                    format!("{depth:.0} GP of pool bid depth, {required:.0} required"),
                );
            }
        }

        Decision::Approve(Order {
            market: view.key().to_string(),
            side: signal.side,
            items,
            limit_price: signal.limit_price,
            strategy: strategy.to_string(),
            reason: signal.reason.clone(),
        })
    }

    /// Record a fill so cooldowns and rate limits see it.
    pub fn record_fill(&mut self, fill: &Fill) {
        self.last_trade.insert(fill.market.clone(), fill.ts);
        self.recent_trades.push_back(fill.ts);
        self.expire_trade_window(fill.ts);
    }

    fn expire_trade_window(&mut self, now: i64) {
        while let Some(front) = self.recent_trades.front() {
            if now - front >= 3_600 {
                self.recent_trades.pop_front();
            } else {
                break;
            }
        }
    }

    fn quote(
        &self,
        view: &MarketView<'_>,
        side: Side,
        items: u64,
    ) -> Option<mercantile_dex::quote::Quote> {
        match side {
            Side::Buy => view.quote_buy(items),
            Side::Sell => view.quote_sell(items),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::Fill;
    use crate::testing::TestMarket;

    fn manager(config: RiskConfig) -> RiskManager {
        RiskManager::new(config)
    }

    fn fill(market: &str, ts: i64) -> Fill {
        Fill {
            ts,
            market: market.to_string(),
            side: Side::Buy,
            items: 1,
            gp: 100.0,
            price: 100.0,
            fee_gp: 1.0,
            signature: None,
            paper: true,
            strategy: "test".to_string(),
        }
    }

    fn approve(decision: Decision) -> Order {
        match decision {
            Decision::Approve(order) => order,
            Decision::Reject(reason) => panic!("expected approval, got {reason}"),
        }
    }

    fn rejection(decision: Decision) -> RejectReason {
        match decision {
            Decision::Reject(reason) => reason,
            Decision::Approve(order) => panic!("expected rejection, got {order:?}"),
        }
    }

    #[test]
    fn a_reasonable_buy_is_approved() {
        let mut risk = manager(RiskConfig::default());
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        let order = approve(risk.evaluate(
            "test",
            &Signal::buy(5, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(order.items, 5);
        assert_eq!(order.side, Side::Buy);
    }

    #[test]
    fn orders_are_shrunk_to_fit_the_budget_rather_than_refused() {
        let mut risk = manager(RiskConfig {
            max_gp_per_trade: 150.0, // roughly two lobsters
            min_gp_per_trade: 10.0,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        let order = approve(risk.evaluate(
            "test",
            &Signal::buy(10, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert!(order.items < 10 && order.items > 0, "{}", order.items);
    }

    #[test]
    fn dust_trades_are_refused() {
        let mut risk = manager(RiskConfig {
            min_gp_per_trade: 10_000.0,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(1, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "min_gp_per_trade");
    }

    #[test]
    fn the_cooldown_blocks_a_second_trade_in_the_same_market() {
        let mut risk = manager(RiskConfig {
            trade_cooldown_secs: 300,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        risk.record_fill(&fill("lobster", market.now - 60));
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(1, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "cooldown");
    }

    #[test]
    fn the_hourly_rate_limit_holds_across_markets() {
        let mut risk = manager(RiskConfig {
            max_trades_per_hour: 2,
            trade_cooldown_secs: 0,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        risk.record_fill(&fill("shark", market.now - 100));
        risk.record_fill(&fill("bronze_dagger", market.now - 50));
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(1, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "rate_limit");
    }

    #[test]
    fn old_trades_fall_out_of_the_rate_window() {
        let mut risk = manager(RiskConfig {
            max_trades_per_hour: 1,
            trade_cooldown_secs: 0,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        risk.record_fill(&fill("shark", market.now - 4_000));
        approve(risk.evaluate(
            "test",
            &Signal::buy(1, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
    }

    #[test]
    fn running_out_of_sol_stops_trading_before_a_failed_transaction() {
        let mut risk = manager(RiskConfig::default());
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(1, "why"),
            &market.view(),
            &book,
            Some(0.0001),
        ));
        assert_eq!(reason.rule, "min_sol_balance");
    }

    #[test]
    fn selling_without_a_position_is_refused() {
        let mut risk = manager(RiskConfig::default());
        let market = TestMarket::above_floor(1.4);
        let book = Portfolio::new(100_000.0);
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::sell(5, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "no_position");
    }

    #[test]
    fn sells_are_clamped_to_the_position() {
        let mut risk = manager(RiskConfig::default());
        let market = TestMarket::above_floor(1.6).with_position(3, 50.0);
        let book = Portfolio::new(100_000.0);
        let order = approve(risk.evaluate(
            "test",
            &Signal::sell(50, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(order.items, 3);
    }

    #[test]
    fn the_position_cap_stops_further_buying() {
        let mut risk = manager(RiskConfig {
            max_position_items: 5,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1).with_position(5, 50.0);
        let book = Portfolio::new(100_000.0);
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(5, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "max_position_items");
    }

    #[test]
    fn a_big_loss_halts_the_bot_for_good() {
        let mut risk = manager(RiskConfig {
            daily_loss_limit_gp: 100.0,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let mut book = Portfolio::new(100_000.0);
        book.realized_pnl = -250.0;

        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(1, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "daily_loss_limit");
        assert!(risk.halted().is_some());

        // Even a healthy book stays halted: the switch is a latch, not a filter.
        book.realized_pnl = 0.0;
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(1, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "halted");
    }

    #[test]
    fn price_impact_beyond_the_cap_is_refused() {
        let mut risk = manager(RiskConfig {
            max_price_impact_pct: 0.1,
            max_gp_per_trade: 1e9,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(1_000_000.0);
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(60, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "max_price_impact");
    }

    #[test]
    fn a_strategy_limit_price_is_enforced_against_the_real_quote() {
        let mut risk = manager(RiskConfig::default());
        let market = TestMarket::above_floor(1.1);
        let book = Portfolio::new(100_000.0);
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(5, "why").with_limit(1.0),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "limit_price");
    }

    #[test]
    fn exposure_limits_are_measured_across_the_whole_book() {
        let mut risk = manager(RiskConfig {
            max_total_exposure_gp: 100.0,
            ..Default::default()
        });
        let market = TestMarket::above_floor(1.1);
        let mut book = Portfolio::new(100_000.0);
        book.positions.insert(
            "shark".to_string(),
            crate::portfolio::Position {
                items: 10,
                avg_cost_gp: 50.0,
            },
        );
        let reason = rejection(risk.evaluate(
            "test",
            &Signal::buy(5, "why"),
            &market.view(),
            &book,
            Some(1.0),
        ));
        assert_eq!(reason.rule, "no_budget");
    }
}

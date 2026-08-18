//! A framework trading bot for [Mercantile](https://github.com/MidTermDev/mercantile),
//! the on-chain RuneScape economy.
//!
//! The bot polls the Meteora pools that price every tokenised item against GP,
//! runs pluggable strategies over them, sizes what they ask for through a risk
//! manager, and executes either on paper or on chain. Its parts are separable:
//!
//! * [`alch`] — the high-alchemy arbitrage that connects the pools to the game
//! * [`scarcity`] — which items cannot be produced, and are worth converting GP into
//! * [`shopflip`] — buying on chain to sell to an NPC shop, no Magic level needed
//! * [`market`] — pool snapshots, price history and live quoting
//! * [`strategy`] — the [`Strategy`](strategy::Strategy) trait and three built-ins
//! * [`risk`] — the only component that can authorise a trade
//! * [`execution`] — paper and live executors behind one interface
//! * [`portfolio`] — positions, cash and P&L
//! * [`engine`] — the tick loop that drives all of the above
//! * [`journal`] — an append-only JSONL record of every decision
//!
//! Writing a strategy means implementing one method; everything else — sizing,
//! limits, execution, accounting, journalling — is already in place.

pub mod alch;
pub mod config;
pub mod engine;
pub mod execution;
pub mod journal;
pub mod market;
pub mod portfolio;
pub mod risk;
pub mod scarcity;
pub mod shopflip;
pub mod strategy;
pub mod testing;
pub mod thieving;
pub mod universe;

pub use config::{Config, Mode};
pub use engine::{Engine, TickSummary};
pub use execution::{Executor, Fill, LiveExecutor, PaperExecutor};
pub use journal::{Event, Journal, Report};
pub use market::{MarketView, PriceHistory};
pub use portfolio::{Portfolio, Position};
pub use risk::{Decision, Order, RiskManager};
pub use strategy::{Signal, Strategy};

//! Bot configuration, loaded from TOML.
//!
//! Every field has a defensible default so a minimal config file works, but the
//! two that decide whether real money moves — `bot.mode` and `bot.allow_live` —
//! both default to *off*. Turning either on alone is not enough (see
//! [`Config::live_enabled`]): sending real orders takes a deliberate config edit
//! *and* an explicit `--live` on the command line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::strategy::StrategyEntry;

/// The whole bot configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub rpc: RpcConfig,
    pub registry: RegistryConfig,
    pub bot: BotConfig,
    pub wallet: WalletConfig,
    pub universe: UniverseConfig,
    pub risk: RiskConfig,
    pub paper: PaperConfig,
    pub journal: JournalConfig,
    /// Strategies to run, in order. An empty list means the bot only observes.
    pub strategies: Vec<StrategyEntry>,
}

/// Where to talk to the chain.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RpcConfig {
    pub url: String,
    /// `processed`, `confirmed` or `finalized`.
    pub commitment: String,
    /// Priority fee added to swap transactions, in micro-lamports per compute unit.
    pub priority_fee_micro_lamports: u64,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            url: mercantile_core::DEFAULT_MAINNET_RPC.to_string(),
            commitment: "confirmed".to_string(),
            priority_fee_micro_lamports: 0,
        }
    }
}

/// Where the item registry comes from.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RegistryConfig {
    /// Local registry file. Fetched with `mercbot registry sync` if missing.
    pub path: PathBuf,
    /// Source for `mercbot registry sync`.
    pub url: String,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("registry.json"),
            url: mercantile_core::DEFAULT_REGISTRY_URL.to_string(),
        }
    }
}

/// How the bot runs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BotConfig {
    /// `paper` (simulated fills) or `live` (real swaps).
    pub mode: Mode,
    /// Seconds between ticks.
    pub tick_interval_secs: u64,
    /// Price points retained per market for the strategies that need history.
    pub history_capacity: usize,
    /// Second safety catch: live orders also require this to be `true`.
    pub allow_live: bool,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Paper,
            tick_interval_secs: 60,
            history_capacity: 720,
            allow_live: false,
        }
    }
}

/// Paper or live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Paper,
    Live,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Paper => "paper",
            Mode::Live => "live",
        }
    }
}

/// The signing wallet. Only read in live mode.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct WalletConfig {
    /// Path to a Solana keypair JSON file. Never commit this.
    pub keypair: Option<PathBuf>,
}

/// Which markets the bot watches.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UniverseConfig {
    /// Explicit registry keys. When non-empty, the filters below are ignored.
    pub include: Vec<String>,
    /// Registry keys to always skip.
    pub exclude: Vec<String>,
    /// Ignore markets whose floor is below this many GP — the dust end of the
    /// registry, where fees swamp any edge.
    pub min_floor_gp: f64,
    /// Ignore markets whose floor is above this many GP.
    pub max_floor_gp: f64,
    /// Restrict to members-only items (or free-to-play when `false`). `None` keeps both.
    pub members_only: Option<bool>,
    /// Cap on how many markets to watch after filtering.
    pub limit: usize,
}

impl Default for UniverseConfig {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            min_floor_gp: 5.0,
            max_floor_gp: f64::MAX,
            members_only: None,
            limit: 25,
        }
    }
}

/// Hard limits applied to every order, whatever a strategy asks for.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RiskConfig {
    /// Largest GP notional for a single trade.
    pub max_gp_per_trade: f64,
    /// Smallest GP notional worth sending — below this, fees dominate.
    pub min_gp_per_trade: f64,
    /// Cap on items held in one market.
    pub max_position_items: u64,
    /// Cap on GP cost basis held in one market.
    pub max_position_gp_per_market: f64,
    /// Cap on total cost basis across all markets.
    pub max_total_exposure_gp: f64,
    /// Slippage tolerance sent to the program.
    pub max_slippage_pct: f64,
    /// Refuse trades whose quoted price impact exceeds this.
    ///
    /// Mercantile pools are seeded with only 100 items, so impact is large by the
    /// standards of a normal AMM: buying five lobsters moves a real pool about 7%.
    /// A tight cap here silently stops the bot trading at all.
    pub max_price_impact_pct: f64,
    /// Keep at least this much SOL for fees (live mode).
    pub min_sol_balance: f64,
    /// GP held back and never spent.
    pub reserve_gp: f64,
    /// Minimum seconds between trades in the same market.
    pub trade_cooldown_secs: i64,
    /// Cap on trades per rolling hour, across all markets.
    pub max_trades_per_hour: usize,
    /// Stop trading for the session once realised losses reach this. 0 disables.
    pub daily_loss_limit_gp: f64,
    /// Refuse a buy unless the pool could absorb selling the position back,
    /// expressed as a multiple of the trade's GP notional.
    ///
    /// Off by default. Mercantile pools are seeded single-sided at the floor, so
    /// their GP side is only what previous buyers paid in: a pool sitting on its
    /// floor has *no* bid at all, and requiring pool-side depth will stop most
    /// floor buying. Leave it at zero if the plan is to exit through the bridge
    /// and the game (alchemy, shops, players); raise it if the pool is the only
    /// exit you are willing to rely on.
    pub min_exit_depth_mult: f64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_gp_per_trade: 25_000.0,
            min_gp_per_trade: 50.0,
            max_position_items: 200,
            max_position_gp_per_market: 100_000.0,
            max_total_exposure_gp: 500_000.0,
            max_slippage_pct: 1.0,
            max_price_impact_pct: 10.0,
            min_sol_balance: 0.01,
            reserve_gp: 0.0,
            trade_cooldown_secs: 300,
            max_trades_per_hour: 20,
            daily_loss_limit_gp: 0.0,
            min_exit_depth_mult: 0.0,
        }
    }
}

/// Starting balances for paper mode.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PaperConfig {
    pub starting_gp: f64,
    /// Optional opening item positions, keyed by registry key.
    pub starting_items: BTreeMap<String, u64>,
    /// Extra cost applied to every simulated fill, in basis points, to stand in
    /// for latency between quoting and landing.
    pub slippage_bps: f64,
}

impl Default for PaperConfig {
    fn default() -> Self {
        Self {
            starting_gp: 1_000_000.0,
            starting_items: BTreeMap::new(),
            slippage_bps: 10.0,
        }
    }
}

/// Where the append-only run journal is written.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct JournalConfig {
    pub path: PathBuf,
    /// Also journal per-tick market snapshots (larger files, replayable history).
    pub record_snapshots: bool,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("mercbot-journal.jsonl"),
            record_snapshots: false,
        }
    }
}

impl Config {
    /// Load a config from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let config: Config = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Whether real orders may be sent. Both the config switch and the mode must agree;
    /// the CLI adds a third gate (`--live`).
    pub fn live_enabled(&self) -> bool {
        self.bot.mode == Mode::Live && self.bot.allow_live
    }

    /// Reject configurations that would misbehave rather than discovering it at tick 1.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.bot.tick_interval_secs == 0 {
            anyhow::bail!("bot.tick_interval_secs must be at least 1");
        }
        if self.bot.history_capacity < 2 {
            anyhow::bail!("bot.history_capacity must be at least 2");
        }
        if self.risk.max_slippage_pct < 0.0 || self.risk.max_slippage_pct > 50.0 {
            anyhow::bail!("risk.max_slippage_pct must be between 0 and 50");
        }
        if self.risk.min_gp_per_trade > self.risk.max_gp_per_trade {
            anyhow::bail!("risk.min_gp_per_trade exceeds risk.max_gp_per_trade");
        }
        if self.universe.min_floor_gp > self.universe.max_floor_gp {
            anyhow::bail!("universe.min_floor_gp exceeds universe.max_floor_gp");
        }
        if self.bot.mode == Mode::Live && self.wallet.keypair.is_none() {
            anyhow::bail!("bot.mode = \"live\" requires wallet.keypair");
        }
        for entry in &self.strategies {
            entry.validate()?;
        }
        Ok(())
    }

    /// The commitment level, defaulting to `confirmed` on anything unrecognised.
    pub fn commitment(&self) -> solana_commitment_config::CommitmentConfig {
        use solana_commitment_config::CommitmentConfig;
        match self.rpc.commitment.as_str() {
            "processed" => CommitmentConfig::processed(),
            "finalized" => CommitmentConfig::finalized(),
            _ => CommitmentConfig::confirmed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let config = Config::default();
        assert_eq!(config.bot.mode, Mode::Paper);
        assert!(!config.bot.allow_live);
        assert!(!config.live_enabled());
        config.validate().unwrap();
    }

    #[test]
    fn live_needs_both_switches_and_a_keypair() {
        let mut config = Config::default();
        config.bot.mode = Mode::Live;
        assert!(
            config.validate().is_err(),
            "live without a keypair is rejected"
        );

        config.wallet.keypair = Some(PathBuf::from("/tmp/id.json"));
        config.validate().unwrap();
        assert!(!config.live_enabled(), "allow_live is still off");

        config.bot.allow_live = true;
        assert!(config.live_enabled());
    }

    #[test]
    fn a_minimal_config_parses() {
        let config: Config = toml::from_str(
            r#"
            [bot]
            tick_interval_secs = 30

            [[strategies]]
            kind = "alch-floor"
            "#,
        )
        .unwrap();
        assert_eq!(config.bot.tick_interval_secs, 30);
        assert_eq!(config.strategies.len(), 1);
        assert_eq!(config.rpc.url, mercantile_core::DEFAULT_MAINNET_RPC);
        config.validate().unwrap();
    }

    #[test]
    fn typos_are_rejected_rather_than_silently_ignored() {
        let err = toml::from_str::<Config>(
            r#"
            [risk]
            max_gp_per_trader = 100
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("max_gp_per_trader"), "{err}");
    }

    #[test]
    fn contradictory_limits_are_caught_up_front() {
        let mut config = Config::default();
        config.risk.min_gp_per_trade = 1_000.0;
        config.risk.max_gp_per_trade = 100.0;
        assert!(config.validate().is_err());
    }
}

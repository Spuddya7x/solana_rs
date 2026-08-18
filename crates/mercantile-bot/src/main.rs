//! `mercbot` — a framework trading bot for the Mercantile on-chain RuneScape economy.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use mercantile_bot::alch::{best_size, rank, RuneCost};
use mercantile_bot::config::{Config, Mode};
use mercantile_bot::engine::{BalanceSource, Engine, NoBalances};
use mercantile_bot::execution::{Executor, LiveExecutor, PaperExecutor};
use mercantile_bot::journal::{Journal, Report};
use mercantile_bot::portfolio::Portfolio;
use mercantile_bot::risk::RiskManager;
use mercantile_bot::universe;
use mercantile_core::{Market, Registry};
use mercantile_dex::client::{ChainClient, PoolFetch};
use mercantile_dex::pool::PoolState;
use mercantile_dex::quote::{Side, SwapQuote};
use solana_sdk::pubkey::Pubkey;

/// A framework trading bot for Mercantile, the on-chain RuneScape economy.
#[derive(Parser)]
#[command(name = "mercbot", version, about, long_about = None)]
struct Cli {
    /// Config file. Commands that only read the chain fall back to defaults.
    #[arg(short, long, default_value = "mercbot.toml", global = true)]
    config: PathBuf,

    /// Log level: error, warn, info, debug, trace.
    #[arg(long, default_value = "info", global = true)]
    log: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List tradeable markets with live prices.
    Markets(MarketsArgs),
    /// Quote a trade against a live pool.
    Quote(QuoteArgs),
    /// Show a wallet's GP and item holdings.
    Balances(BalancesArgs),
    /// Rank markets by high-alchemy arbitrage profit.
    AlchScan(AlchScanArgs),
    /// Plan a Magic training route to the level alchemy needs.
    MagicPlan(MagicPlanArgs),
    /// Rank markets by how little of the item can ever exist.
    Scarcity(ScarcityArgs),
    /// Run the bot.
    Run(RunArgs),
    /// Download the item registry.
    RegistrySync(RegistrySyncArgs),
    /// Summarise a run journal.
    Report(ReportArgs),
    /// Describe the built-in strategies.
    Strategies,
}

#[derive(Args)]
struct MarketsArgs {
    /// How many markets to show.
    #[arg(long, default_value_t = 25)]
    limit: usize,
    /// Sort by: premium, price, floor, depth.
    #[arg(long, default_value = "premium")]
    sort: String,
    /// Only markets at or below this multiple of their floor.
    #[arg(long)]
    max_premium: Option<f64>,
    /// Emit JSON instead of a table.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct QuoteArgs {
    /// Registry key, symbol, name or mint.
    item: String,
    /// buy or sell.
    side: String,
    /// Whole items.
    count: u64,
    /// Slippage tolerance for the reported guard rails.
    #[arg(long, default_value_t = 1.0)]
    slippage: f64,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct BalancesArgs {
    /// Wallet address. Defaults to the configured keypair's public key.
    address: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct AlchScanArgs {
    /// Largest stack to consider per market.
    #[arg(long, default_value_t = 25)]
    max_items: u64,
    /// GP per nature rune. Defaults to the live nature rune pool price.
    #[arg(long)]
    nature_rune_gp: Option<f64>,
    /// GP per fire rune. Zero assumes a staff of fire.
    #[arg(long, default_value_t = 0.0)]
    fire_rune_gp: f64,
    /// How many opportunities to show.
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Only scan this many of the richest markets (each one costs an RPC read).
    #[arg(long, default_value_t = 200)]
    scan: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ScarcityArgs {
    /// Whole items to price per market.
    #[arg(long, default_value_t = 1)]
    items: u64,
    /// How many to show.
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Include items the game can still produce.
    #[arg(long)]
    all: bool,
    /// Ignore markets floored below this many GP — most unprintable items are
    /// macro-event cubes and quest litter at 0.9 GP.
    #[arg(long, default_value_t = 100.0)]
    min_floor: f64,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct MagicPlanArgs {
    /// Level to start from.
    #[arg(long, default_value_t = 1)]
    from: u32,
    /// Level to reach. 55 unlocks high alchemy; 66 unlocks the Wizards' Guild
    /// and with it an unbounded nature rune supply.
    #[arg(long, default_value_t = 55)]
    to: u32,
    /// cheapest, fastest, or both.
    #[arg(long, default_value = "both")]
    route: String,
    /// Allow spells needing runes no open shop sells (nature, law, blood, soul).
    #[arg(long)]
    any_runes: bool,
    /// Assume no undead target is available, ruling out crumble undead.
    #[arg(long)]
    no_undead: bool,
    /// Allow alchemy legs. Needs nature runes and a stream of items, both
    /// supply-limited until the Wizards' Guild opens at 66.
    #[arg(long)]
    with_alchemy: bool,
    /// GP per nature rune. Defaults to the live pool price when alchemy is on.
    #[arg(long)]
    nature_rune_gp: Option<f64>,
    /// Assume a staff of fire, which makes fire runes free.
    #[arg(long)]
    fire_staff: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct RunArgs {
    /// Run a single tick and exit.
    #[arg(long)]
    once: bool,
    /// Third and final gate for real orders, on top of `bot.mode` and `bot.allow_live`.
    #[arg(long)]
    live: bool,
}

#[derive(Args)]
struct RegistrySyncArgs {
    /// Source URL. Defaults to the configured one.
    #[arg(long)]
    url: Option<String>,
    /// Where to write it. Defaults to the configured registry path.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Args)]
struct ReportArgs {
    /// Journal file. Defaults to the configured one.
    journal: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("mercbot={0},mercantile_bot={0},mercantile_dex={0}", cli.log).into()
            }),
        )
        .with_target(false)
        .init();

    match &cli.command {
        Command::Markets(args) => markets(&cli, args),
        Command::Quote(args) => quote(&cli, args),
        Command::Balances(args) => balances(&cli, args),
        Command::AlchScan(args) => alch_scan(&cli, args),
        Command::MagicPlan(args) => magic_plan(&cli, args),
        Command::Scarcity(args) => scarcity(&cli, args),
        Command::Run(args) => run(&cli, args),
        Command::RegistrySync(args) => registry_sync(&cli, args),
        Command::Report(args) => report(&cli, args),
        Command::Strategies => {
            strategies();
            Ok(())
        }
    }
}

/// Load the config, tolerating a missing file for read-only commands.
fn load_config(cli: &Cli) -> Result<Config> {
    if cli.config.exists() {
        Config::load(&cli.config)
    } else {
        Ok(Config::default())
    }
}

fn load_registry(config: &Config) -> Result<Registry> {
    if config.registry.path.exists() {
        return Registry::load(&config.registry.path).map_err(Into::into);
    }
    tracing::info!(url = %config.registry.url, "no local registry, downloading");
    let (registry, text) = Registry::fetch(Some(&config.registry.url))?;
    if let Err(err) = std::fs::write(&config.registry.path, &text) {
        tracing::warn!(%err, path = %config.registry.path.display(), "could not cache the registry");
    }
    Ok(registry)
}

fn client(config: &Config) -> ChainClient {
    ChainClient::new(config.rpc.url.clone(), config.commitment())
        .with_priority_fee(config.rpc.priority_fee_micro_lamports)
}

fn markets(cli: &Cli, args: &MarketsArgs) -> Result<()> {
    let config = load_config(cli)?;
    let registry = load_registry(&config)?;
    let chain = client(&config);

    // `markets` is a browsing command: it shows the whole registry unless the
    // config names an explicit universe, in which case that is clearly what the
    // operator wants to look at.
    let mut selected = if config.universe.include.is_empty() {
        let mut all = registry.markets()?;
        all.sort_by(|a, b| {
            b.floor_gp()
                .partial_cmp(&a.floor_gp())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all
    } else {
        universe::select(&registry, &config.universe)?
    };
    // Fetching pools costs an RPC read each, so bound the scan while still
    // leaving room for the premium filter to discard most of them.
    selected.truncate((args.limit.max(1) * 4).max(64));

    let pools: Vec<Pubkey> = selected.iter().map(|m| m.pool).collect();
    let states = chain.fetch_pools(&pools)?;

    let mut rows: Vec<MarketRow> = selected
        .iter()
        .zip(states)
        .filter_map(|(market, state)| state.map(|pool| MarketRow::new(market, &pool)))
        .filter(|row| args.max_premium.is_none_or(|max| row.premium <= max))
        .collect();

    match args.sort.as_str() {
        "price" => rows.sort_by(|a, b| b.price.total_cmp(&a.price)),
        "floor" => rows.sort_by(|a, b| b.floor.total_cmp(&a.floor)),
        "depth" => rows.sort_by(|a, b| b.exit_depth_gp.total_cmp(&a.exit_depth_gp)),
        _ => rows.sort_by(|a, b| a.premium.total_cmp(&b.premium)),
    }
    rows.truncate(args.limit);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!(
        "{:<22} {:>10} {:>10} {:>8} {:>10} {:>8}",
        "market", "price", "floor", "prem", "bid depth", "items"
    );
    for row in &rows {
        println!(
            "{:<22} {:>10.2} {:>10.2} {:>7.2}x {:>10.0} {:>8.0}",
            row.market, row.price, row.floor, row.premium, row.exit_depth_gp, row.pool_items
        );
    }
    println!(
        "\n{} markets priced. 'prem' is spot as a multiple of the permanent alch floor;",
        rows.len()
    );
    println!("'bid depth' is the GP a pool can pay out before it reaches that floor.");
    Ok(())
}

#[derive(serde::Serialize)]
struct MarketRow {
    market: String,
    name: String,
    price: f64,
    floor: f64,
    premium: f64,
    exit_depth_gp: f64,
    pool_items: f64,
    mint: String,
    pool: String,
}

impl MarketRow {
    fn new(market: &Market, pool: &PoolState) -> Self {
        Self {
            market: market.key.clone(),
            name: market.item.name.clone(),
            price: pool.spot_price(),
            floor: pool.floor_price(),
            premium: pool.premium_over_floor(),
            exit_depth_gp: pool
                .gp_reserve()
                .map(|b| mercantile_core::base_to_gp(b.min(u64::MAX as u128) as u64))
                .unwrap_or(0.0),
            pool_items: pool
                .item_reserve()
                .map(|b| mercantile_core::base_to_items(b.min(u64::MAX as u128) as u64))
                .unwrap_or(0.0),
            mint: market.mint.to_string(),
            pool: market.pool.to_string(),
        }
    }
}

fn quote(cli: &Cli, args: &QuoteArgs) -> Result<()> {
    let config = load_config(cli)?;
    let registry = load_registry(&config)?;
    let chain = client(&config);

    let side = match args.side.to_ascii_lowercase().as_str() {
        "buy" => Side::Buy,
        "sell" => Side::Sell,
        other => anyhow::bail!("side must be buy or sell, not {other:?}"),
    };
    let market = registry.market(&args.item)?;
    let pool = chain.fetch_pool(&market.pool)?;
    let point = chain.current_point()?;

    let quote = match side {
        Side::Buy => pool.quote_buy_items(args.count, point),
        Side::Sell => pool.quote_sell_items(args.count, point),
    }
    .with_context(|| format!("quoting {} {} x{}", args.side, market.key, args.count))?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "market": market.key,
                "side": args.side,
                "items": args.count,
                "gp": quote.gp(),
                "execution_price": quote.execution_price,
                "spot_price": quote.spot_price,
                "floor": pool.floor_price(),
                "price_impact_pct": quote.price_impact_pct,
                "fee_gp": mercantile_core::base_to_gp(quote.trade_fee.min(u64::MAX as u128) as u64),
                "max_amount_in": quote.max_amount_in(args.slippage),
                "min_amount_out": quote.min_amount_out(args.slippage),
                "high_alch_gp": mercantile_core::high_alch_value(market.item.cost) * args.count,
            })
        );
        return Ok(());
    }

    let alch = mercantile_core::high_alch_value(market.item.cost) * args.count;
    println!("{} ({})", market.item.name, market.key);
    println!("  spot            {:.4} GP/item", quote.spot_price);
    println!("  floor           {:.4} GP/item", pool.floor_price());
    println!(
        "  {} {:<10} {:.4} GP total, {:.4} GP/item",
        args.side,
        args.count,
        quote.gp(),
        quote.execution_price
    );
    println!("  price impact    {:.2}%", quote.price_impact_pct);
    println!(
        "  pool fee        {:.4} GP",
        mercantile_core::base_to_gp(quote.trade_fee.min(u64::MAX as u128) as u64)
    );
    if side == Side::Buy {
        println!(
            "  high alch value {alch} GP ({} GP/item)",
            mercantile_core::high_alch_value(market.item.cost)
        );
        println!(
            "  alch margin     {:+.2} GP before runes",
            alch as f64 - quote.gp()
        );
    }
    Ok(())
}

fn balances(cli: &Cli, args: &BalancesArgs) -> Result<()> {
    let config = load_config(cli)?;
    let registry = load_registry(&config)?;
    let chain = client(&config);

    let owner: Pubkey = match &args.address {
        Some(address) => address.parse().context("parsing the wallet address")?,
        None => {
            let path = config
                .wallet
                .keypair
                .as_ref()
                .context("no address given and no wallet.keypair configured")?;
            let keypair = solana_sdk::signature::read_keypair_file(path)
                .map_err(|e| anyhow::anyhow!("reading keypair {}: {e}", path.display()))?;
            solana_sdk::signer::Signer::pubkey(&keypair)
        }
    };

    let gp = mercantile_core::base_to_gp(chain.gp_balance(&owner)?);
    let sol = chain.sol_balance(&owner)? as f64 / 1e9;
    let items = chain.item_balances(&owner)?;

    let by_mint: std::collections::HashMap<String, &Market> = Default::default();
    let markets = registry.markets()?;
    let mut by_mint = by_mint;
    for market in &markets {
        by_mint.insert(market.mint.to_string(), market);
    }

    let mut rows: Vec<(String, f64)> = items
        .into_iter()
        .filter_map(|(mint, amount)| {
            by_mint
                .get(&mint.to_string())
                .map(|market| (market.key.clone(), mercantile_core::base_to_items(amount)))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    if args.json {
        println!(
            "{}",
            serde_json::json!({ "owner": owner.to_string(), "sol": sol, "gp": gp, "items": rows })
        );
        return Ok(());
    }
    println!("{owner}");
    println!("  {:<22} {:>14.4}", "SOL", sol);
    println!("  {:<22} {:>14.2}", "GP", gp);
    for (key, count) in &rows {
        println!("  {key:<22} {count:>14}");
    }
    if rows.is_empty() {
        println!("  (no item tokens)");
    }
    Ok(())
}

fn alch_scan(cli: &Cli, args: &AlchScanArgs) -> Result<()> {
    let config = load_config(cli)?;
    let registry = load_registry(&config)?;
    let chain = client(&config);
    let point = chain.current_point()?;

    // Nature runes are the per-cast cost, so price them first.
    let nature_rune_gp = match args.nature_rune_gp {
        Some(price) => price,
        None => match registry.market(mercantile_bot::strategy::alch_arb::NATURE_RUNE_KEY) {
            Ok(runes) => match chain.fetch_pool(&runes.pool) {
                Ok(pool) => pool.spot_price(),
                Err(err) => {
                    tracing::warn!(%err, "could not price nature runes, using the pool floor");
                    7.2
                }
            },
            Err(err) => {
                tracing::warn!(%err, "no nature rune market in the registry, using its floor");
                7.2
            }
        },
    };
    let runes = RuneCost {
        nature_rune_gp,
        fire_rune_gp: args.fire_rune_gp,
    };

    // High alchemy pays 0.6 x cost, so richer items carry more profit per cast:
    // scan from the top of the registry down.
    let mut candidates = registry.markets()?;
    candidates.sort_by(|a, b| b.item.cost.cmp(&a.item.cost));
    candidates.truncate(args.scan);

    let pools: Vec<Pubkey> = candidates.iter().map(|m| m.pool).collect();
    let states = chain.fetch_pools(&pools)?;

    let opportunities: Vec<_> = candidates
        .iter()
        .zip(states)
        .filter_map(|(market, state)| {
            let pool = state?;
            best_size(market, &pool, args.max_items, runes, point)
        })
        .collect();
    let ranked = rank(opportunities);
    let shown: Vec<_> = ranked.iter().take(args.limit).collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&shown)?);
        return Ok(());
    }

    println!(
        "nature rune {nature_rune_gp:.2} GP, fire rune {:.2} GP -> {:.2} GP per cast",
        args.fire_rune_gp,
        runes.per_high_alch()
    );
    println!(
        "{:<22} {:>5} {:>10} {:>10} {:>10} {:>7} {:>11}",
        "market", "items", "buy GP", "alch GP", "profit", "margin", "GP/hour"
    );
    for row in &shown {
        println!(
            "{:<22} {:>5} {:>10.0} {:>10.0} {:>10.0} {:>6.0}% {:>11.0}",
            row.market,
            row.items,
            row.gp_cost,
            row.alch_gp,
            row.profit_gp,
            row.margin * 100.0,
            row.profit_per_hour
        );
    }
    if shown.is_empty() {
        println!("(nothing profitable at these rune prices)");
    } else {
        println!(
            "\n{} of the {} richest markets are profitable to alch, at one cast per {:.1}s.",
            ranked.len(),
            candidates.len(),
            mercantile_core::alch::seconds_per_high_alch()
        );
        println!(
            "'GP/hour' is casting-limited and assumes the stack is already bought. In practice"
        );
        println!(
            "pool inventory binds first: each pool holds about a hundred items, and 'items' above"
        );
        println!("is where buying one more costs more than alching it returns.");
    }
    Ok(())
}

fn scarcity(cli: &Cli, args: &ScarcityArgs) -> Result<()> {
    use mercantile_bot::scarcity::{assess, rank};

    let config = load_config(cli)?;
    let registry = load_registry(&config)?;
    let chain = client(&config);
    let point = chain.current_point()?;

    // Only the items with no in-game source are worth the RPC reads, unless the
    // caller wants the whole registry.
    let markets: Vec<Market> = registry
        .markets()?
        .into_iter()
        .filter(|m| args.all || mercantile_core::sources_for(&m.key).capped)
        .collect();
    anyhow::ensure!(!markets.is_empty(), "no markets matched");

    let pools: Vec<Pubkey> = markets.iter().map(|m| m.pool).collect();
    let mints: Vec<Pubkey> = markets.iter().map(|m| m.mint).collect();
    let states = chain.fetch_pools(&pools)?;
    let supplies = chain.token_supplies(&mints)?;

    let rows: Vec<_> = markets
        .iter()
        .zip(states)
        .zip(supplies)
        .filter_map(|((market, state), supply)| {
            Some(assess(market, &state?, supply?, args.items, point))
        })
        .collect();
    let ranked = rank(rows);
    let shown: Vec<_> = ranked
        .iter()
        .filter(|row| row.floor >= args.min_floor)
        .take(args.limit)
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&shown)?);
        return Ok(());
    }

    println!(
        "{:<24} {:>7} {:>9} {:>6} {:>12} {:>12}  source",
        "market", "supply", "bridged", "score", "price", "buy GP"
    );
    for row in &shown {
        println!(
            "{:<24} {:>7.0} {:>9.0} {:>6.2} {:>12.0} {:>12}  {}",
            row.market,
            row.supply,
            row.bridged_in,
            row.score,
            row.price,
            row.gp_cost
                .map(|gp| format!("{gp:.0}"))
                .unwrap_or_else(|| "-".into()),
            row.sources.describe(),
        );
    }
    println!(
        "\n{} of {} markets scanned. Every pool was seeded with {} units, so 'bridged' is",
        shown.len(),
        markets.len(),
        mercantile_core::POOL_SEED_UNITS
    );
    println!("supply players have produced in game — direct evidence an item is farmable.");
    println!("Score weights 'nothing in the game makes this' above price; run with --all to");
    println!("include the 1,312 items that something does.");
    Ok(())
}

fn magic_plan(cli: &Cli, args: &MagicPlanArgs) -> Result<()> {
    use mercantile_core::magic::{plan, Constraints, Objective, RunePrices};

    let mut prices = RunePrices::default();
    if args.fire_staff {
        prices = prices.with_fire_staff();
    }
    // Nature runes are the one rune with no open shop, so their price is the
    // live pool price rather than a shop cost.
    if args.with_alchemy {
        let nature_rune_gp = match args.nature_rune_gp {
            Some(price) => Some(price),
            None => {
                let config = load_config(cli)?;
                let registry = load_registry(&config)?;
                let chain = client(&config);
                registry
                    .market(mercantile_bot::strategy::alch_arb::NATURE_RUNE_KEY)
                    .ok()
                    .and_then(|market| chain.fetch_pool(&market.pool).ok())
                    .map(|pool| pool.spot_price())
            }
        };
        if let Some(price) = nature_rune_gp {
            prices.set("naturerune", price);
        }
    } else if let Some(price) = args.nature_rune_gp {
        prices.set("naturerune", price);
    }

    let constraints = Constraints {
        shop_runes_only: !args.any_runes,
        allow_undead: !args.no_undead,
        allow_alchemy: args.with_alchemy,
    };

    let objectives: Vec<(&str, Objective)> = match args.route.as_str() {
        "cheapest" => vec![("cheapest", Objective::Cheapest)],
        "fastest" => vec![("fastest", Objective::Fastest)],
        _ => vec![
            ("cheapest", Objective::Cheapest),
            ("fastest", Objective::Fastest),
        ],
    };

    let mut json_routes = Vec::new();
    for (label, objective) in objectives {
        let plan = plan(args.from, args.to, objective, &prices, &constraints);
        if args.json {
            json_routes.push(serde_json::json!({
                "route": label,
                "from": args.from,
                "to": args.to,
                "casts": plan.casts(),
                "hours": plan.hours(),
                "rune_gp": plan.rune_gp(),
                "legs": plan.legs.iter().map(|leg| serde_json::json!({
                    "spell": leg.spell.name,
                    "component": leg.spell.component,
                    "from_level": leg.from_level,
                    "to_level": leg.to_level,
                    "casts": leg.casts,
                    "hours": leg.seconds / 3600.0,
                    "rune_gp": leg.rune_gp,
                    "xp_per_cast": leg.spell.xp,
                    "splash": leg.spell.repeatability
                        == mercantile_core::magic::Repeatability::RequiresSplash,
                })).collect::<Vec<_>>(),
            }));
            continue;
        }

        println!("{label} route, magic {} to {}:", args.from, args.to);
        println!(
            "  {:<16} {:>7} {:>9} {:>7} {:>10} {:>8}",
            "spell", "levels", "casts", "hours", "rune GP", "cast on"
        );
        for leg in &plan.legs {
            let target = match leg.spell.target {
                mercantile_core::magic::TargetRequirement::Undead => "undead",
                mercantile_core::magic::TargetRequirement::InventoryItem => "an item",
                mercantile_core::magic::TargetRequirement::Any => {
                    if leg.spell.repeatability
                        == mercantile_core::magic::Repeatability::RequiresSplash
                    {
                        "splash"
                    } else {
                        "any npc"
                    }
                }
            };
            println!(
                "  {:<16} {:>3}-{:<3} {:>9} {:>7.1} {:>10.0} {:>8}",
                leg.spell.name,
                leg.from_level,
                leg.to_level,
                leg.casts,
                leg.seconds / 3600.0,
                leg.rune_gp,
                target
            );
        }
        println!(
            "  total: {} casts, {:.1} hours, {:.0} GP of runes\n",
            plan.casts(),
            plan.hours(),
            plan.rune_gp()
        );
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&json_routes)?);
    } else {
        println!("Every combat spell casts in 5 ticks, so 'fastest' is simply the most XP per");
        println!("cast; only low alchemy (3 ticks) breaks that. 'splash' legs are debuff spells");
        println!("that a landed cast would block — wear a bronze kit (-69 magic attack) so every");
        println!("cast misses. Rune prices are the game's shop values; pass --fire-staff if one");
        println!("is equipped.");
    }
    Ok(())
}

fn run(cli: &Cli, args: &RunArgs) -> Result<()> {
    let config = Config::load(&cli.config).with_context(|| {
        format!(
            "`run` needs a config file; {} not found",
            cli.config.display()
        )
    })?;
    let registry = load_registry(&config)?;
    let markets = universe::select(&registry, &config.universe)?;
    anyhow::ensure!(
        !markets.is_empty(),
        "the configured universe selected no markets"
    );

    let strategies: Vec<_> = config
        .strategies
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| entry.build())
        .collect();
    anyhow::ensure!(!strategies.is_empty(), "no strategies are enabled");

    let chain = client(&config);
    let risk = RiskManager::new(config.risk.clone());
    let journal = Journal::open(&config.journal.path)?;

    // The accumulate strategy trades on supply, so read it up front. It only
    // moves when someone bridges an item out of the game, so once is plenty for
    // a short run and `with_supply_refresh` covers long ones.
    let supplies = read_supplies(&client(&config), &markets);

    // Three independent gates protect live trading. Any one of them missing
    // keeps the run on paper.
    let live = args.live && config.live_enabled();
    if args.live && !live {
        anyhow::bail!(
            "--live needs bot.mode = \"live\" and bot.allow_live = true in {}",
            cli.config.display()
        );
    }

    if live {
        let keypair_path = config
            .wallet
            .keypair
            .clone()
            .context("live mode needs wallet.keypair")?;
        let executor =
            LiveExecutor::new(client(&config), &keypair_path, config.risk.max_slippage_pct)?;
        let owner = executor.pubkey();
        tracing::warn!(%owner, markets = markets.len(), "LIVE — real orders will be sent");
        let balances = ChainBalances {
            client: client(&config),
            owner,
        };
        let (gp, items) = balances.balances(&markets)?;
        let mut portfolio = Portfolio::new(gp);
        portfolio.sync_balances(gp, &items);
        let mut engine = Engine::new(
            markets,
            chain,
            executor,
            balances,
            strategies,
            risk,
            portfolio,
            config.bot.history_capacity,
            journal,
        )
        .with_snapshots(config.journal.record_snapshots)
        .with_balance_sync(true)
        .with_supplies(supplies)
        .with_supply_refresh(60);
        drive(&mut engine, &config, args.once)
    } else {
        if config.bot.mode == Mode::Live {
            tracing::warn!("config says live but --live was not passed; running on paper");
        }
        let mut portfolio = Portfolio::new(config.paper.starting_gp);
        for (key, items) in &config.paper.starting_items {
            portfolio.positions.insert(
                key.clone(),
                mercantile_bot::portfolio::Position {
                    items: *items,
                    avg_cost_gp: 0.0,
                },
            );
        }
        tracing::info!(
            markets = markets.len(),
            gp = config.paper.starting_gp,
            "paper mode — no transactions will be sent"
        );
        let mut engine = Engine::new(
            markets,
            chain,
            PaperExecutor::new(config.paper.slippage_bps),
            NoBalances,
            strategies,
            risk,
            portfolio,
            config.bot.history_capacity,
            journal,
        )
        .with_snapshots(config.journal.record_snapshots)
        .with_supplies(supplies)
        .with_supply_refresh(60);
        drive(&mut engine, &config, args.once)
    }
}

/// Token supply per market, in whole items. Failures are logged, not fatal:
/// without supply the scarcity strategies abstain, which is the safe default.
fn read_supplies(
    chain: &ChainClient,
    markets: &[Market],
) -> std::collections::BTreeMap<String, f64> {
    let mints: Vec<Pubkey> = markets.iter().map(|m| m.mint).collect();
    match chain.token_supplies(&mints) {
        Ok(supplies) => markets
            .iter()
            .zip(supplies)
            .filter_map(|(market, supply)| Some((market.key.clone(), supply?)))
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "could not read token supplies; scarcity strategies will abstain");
            Default::default()
        }
    }
}

fn drive<F: PoolFetch, E: Executor, B: BalanceSource>(
    engine: &mut Engine<F, E, B>,
    config: &Config,
    once: bool,
) -> Result<()> {
    if once {
        let summary = engine.tick()?;
        println!("{summary:#?}");
        println!("gp {:.2}", engine.portfolio().gp);
        return Ok(());
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = shutdown.clone();
    ctrlc::set_handler(move || {
        // Second Ctrl-C is the operator insisting; let the process die.
        if flag.swap(true, Ordering::Relaxed) {
            std::process::exit(130);
        }
        eprintln!("\nstopping after this tick — press Ctrl-C again to quit now");
    })
    .context("installing the Ctrl-C handler")?;

    engine.run(Duration::from_secs(config.bot.tick_interval_secs), shutdown)
}

/// Live balances read from the chain.
struct ChainBalances {
    client: ChainClient,
    owner: Pubkey,
}

impl BalanceSource for ChainBalances {
    fn balances(
        &self,
        markets: &[Market],
    ) -> Result<(f64, std::collections::BTreeMap<String, u64>)> {
        let gp = mercantile_core::base_to_gp(self.client.gp_balance(&self.owner)?);
        let held = self.client.item_balances(&self.owner)?;
        let by_mint: std::collections::HashMap<String, &Market> = markets
            .iter()
            .map(|market| (market.mint.to_string(), market))
            .collect();
        let mut items = std::collections::BTreeMap::new();
        for (mint, amount) in held {
            if let Some(market) = by_mint.get(&mint.to_string()) {
                items.insert(
                    market.key.clone(),
                    mercantile_core::base_to_items(amount).floor() as u64,
                );
            }
        }
        Ok((gp, items))
    }

    fn sol_balance(&self) -> Result<Option<f64>> {
        Ok(Some(self.client.sol_balance(&self.owner)? as f64 / 1e9))
    }
}

fn registry_sync(cli: &Cli, args: &RegistrySyncArgs) -> Result<()> {
    let config = load_config(cli)?;
    let url = args.url.clone().unwrap_or(config.registry.url.clone());
    let out = args.out.clone().unwrap_or(config.registry.path.clone());
    let (registry, text) = Registry::fetch(Some(&url))?;
    std::fs::write(&out, &text).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "{} items ({} listed on chain) written to {}",
        registry.items.len(),
        registry.listed_count(),
        out.display()
    );
    Ok(())
}

fn report(cli: &Cli, args: &ReportArgs) -> Result<()> {
    let config = load_config(cli)?;
    let path = args.journal.clone().unwrap_or(config.journal.path.clone());
    let events = Journal::read(&path)?;
    let report = Report::from_events(&events);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("{}", path.display());
    println!(
        "  fills          {} ({} buys, {} sells)",
        report.fills, report.buys, report.sells
    );
    println!("  GP spent       {:.2}", report.gp_spent);
    println!("  GP received    {:.2}", report.gp_received);
    println!("  fees paid      {:.2}", report.fees_gp);
    println!("  realised P&L   {:+.2}", report.realized_pnl);
    println!("  rejects        {}", report.rejects);
    println!("  errors         {}", report.errors);
    if !report.reject_rules.is_empty() {
        println!("  most common refusals:");
        for (rule, count) in report.reject_rules.iter().take(5) {
            println!("    {rule:<22} {count}");
        }
    }
    if !report.market_activity.is_empty() {
        println!("  busiest markets:");
        for (market, count) in report.market_activity.iter().take(5) {
            println!("    {market:<22} {count}");
        }
    }
    Ok(())
}

fn strategies() {
    println!("built-in strategies (configure with [[strategies]] kind = \"...\"):\n");
    for (name, blurb) in [
        (
            "accumulate",
            "The exit. Converts GP into the 62 items with no in-game source — no drop\n    table, no shop, no skill, no quest, no ground spawn — so their supply cannot\n    grow. Weighs the pool floor too, since most unprintable items are quest\n    litter. Never sells.",
        ),
        (
            "alch-arb",
            "Buys only stacks that High Level Alchemy would pay for: 0.6 x cost per\n    item against the live pool price, less a nature rune. Exits through the\n    Exchange Clerk and the spell, not through the pool.",
        ),
        (
            "alch-floor",
            "Buys near the permanent bid floor (0.9 x lowalch) and sells into strength.\n    The floor cannot be traded through, so downside is bounded — but the pool's\n    bid depth at the floor is zero, so plan the exit.",
        ),
        (
            "mean-reversion",
            "Fades moves away from a rolling mean, gated on the floor premium so it\n    cannot chase something already expensive.",
        ),
        (
            "grid",
            "A ladder of buys and sells at fixed percentage steps above the floor,\n    which is the one anchor in this market that does not drift.",
        ),
    ] {
        println!("  {name}\n    {blurb}\n");
    }
    println!("Every strategy's signals pass through [risk] before execution, so limits");
    println!("live in one place and adding a strategy cannot widen the bot's risk.");
}

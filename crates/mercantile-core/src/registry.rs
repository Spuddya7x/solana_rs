//! The Mercantile item registry: `debugname -> objId / cost / lowalch / mint / pool`.
//!
//! `chain/registry/registry.json` in the Mercantile repository is the source of
//! truth for which items exist on chain and which pool prices them. The bot reads
//! it to build its tradeable universe; nothing here talks to the network except
//! [`Registry::fetch`], which downloads a fresh copy.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::{ParsePubkeyError, Pubkey};

use crate::amounts::floor_price;
use crate::ids::DEFAULT_REGISTRY_URL;

/// Anything that can go wrong loading or interpreting a registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("reading registry from {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing registry JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("downloading registry from {url}: {source}")]
    Download {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },
    #[error("registry entry {item} has a malformed {field} address: {source}")]
    BadAddress {
        item: String,
        field: &'static str,
        #[source]
        source: ParsePubkeyError,
    },
    #[error("no item matches {0:?}")]
    UnknownItem(String),
    #[error("{0} is in the registry but has no pool yet — it is not tradeable on chain")]
    NotListed(String),
}

/// A parsed `registry.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub version: u32,
    #[serde(default)]
    pub network: String,
    /// The AMM program the pools belong to (Meteora `cp_amm`).
    #[serde(default)]
    pub program_id: String,
    #[serde(default)]
    pub generated_at: String,
    pub gp: GpInfo,
    /// Keyed by the game's `debugname`, e.g. `lobster`, `rune_scimitar`.
    pub items: BTreeMap<String, Item>,
}

/// The GP token: quote asset for every market.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpInfo {
    pub mint: Option<String>,
    #[serde(default = "default_gp_decimals")]
    pub decimals: u8,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub total_supply: String,
}

fn default_gp_decimals() -> u8 {
    crate::ids::GP_DECIMALS
}

/// One tradeable item. `mint` and `pool` are `None` until the item is rolled out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub obj_id: u32,
    #[serde(default)]
    pub cert_obj_id: Option<u32>,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    /// In-game shop value.
    pub cost: u64,
    /// `max(floor(cost * 0.4), 1)` — drives the pool's permanent bid floor.
    pub lowalch: u64,
    /// Seed price of the pool in GP per item (`0.9 x lowalch`, or an override for rares).
    #[serde(default)]
    pub p0_gp_per_item: Option<f64>,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub stackable: bool,
    #[serde(default)]
    pub members: bool,
    pub mint: Option<String>,
    pub pool: Option<String>,
    #[serde(default)]
    pub flags: Vec<String>,
}

impl Item {
    /// The permanent bid floor in GP per item: the pool's `P0`.
    ///
    /// Prefers the registry's recorded seed price (rares are repriced above
    /// `0.9 x lowalch`) and falls back to the alch formula.
    pub fn floor_gp(&self) -> f64 {
        self.p0_gp_per_item
            .unwrap_or_else(|| floor_price(self.lowalch))
    }

    /// Whether the item has both a mint and a pool, i.e. it can be traded now.
    pub fn is_listed(&self) -> bool {
        self.mint.is_some() && self.pool.is_some()
    }
}

/// A listed item resolved into addresses — everything needed to price and trade it.
#[derive(Debug, Clone)]
pub struct Market {
    /// The registry key (`debugname`), used as the market's identity everywhere.
    pub key: String,
    pub item: Item,
    pub mint: Pubkey,
    pub pool: Pubkey,
}

impl Market {
    /// The permanent bid floor in GP per item.
    pub fn floor_gp(&self) -> f64 {
        self.item.floor_gp()
    }

    /// Display name, e.g. `Lobster`.
    pub fn name(&self) -> &str {
        &self.item.name
    }
}

impl Registry {
    /// Parse a registry from JSON text.
    pub fn from_json(json: &str) -> Result<Self, RegistryError> {
        Ok(serde_json::from_str(json)?)
    }

    /// Load a registry from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| RegistryError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_json(&text)
    }

    /// Download a registry over HTTP. `url` defaults to [`DEFAULT_REGISTRY_URL`].
    pub fn fetch(url: Option<&str>) -> Result<(Self, String), RegistryError> {
        let url = url.unwrap_or(DEFAULT_REGISTRY_URL);
        let download = || -> Result<String, ureq::Error> {
            let mut body = String::new();
            ureq::get(url)
                .call()?
                .body_mut()
                // registry.json is ~1 MB; the default cap is smaller.
                .as_reader()
                .take(64 * 1024 * 1024)
                .read_to_string(&mut body)
                .map_err(ureq::Error::from)?;
            Ok(body)
        };
        let text = download().map_err(|source| RegistryError::Download {
            url: url.to_string(),
            source: Box::new(source),
        })?;
        let registry = Self::from_json(&text)?;
        Ok((registry, text))
    }

    /// The GP mint, if the registry has one recorded.
    pub fn gp_mint(&self) -> Option<Pubkey> {
        self.gp.mint.as_ref().and_then(|m| m.parse().ok())
    }

    /// Every item that has a mint and a pool, resolved into a [`Market`].
    ///
    /// Entries with malformed addresses are an error rather than a silent skip:
    /// a registry the bot cannot fully understand is a registry it should not trade on.
    pub fn markets(&self) -> Result<Vec<Market>, RegistryError> {
        self.items
            .iter()
            .filter(|(_, item)| item.is_listed())
            .map(|(key, item)| self.resolve_market(key, item))
            .collect()
    }

    fn resolve_market(&self, key: &str, item: &Item) -> Result<Market, RegistryError> {
        let mint = item
            .mint
            .as_deref()
            .unwrap_or_default()
            .parse()
            .map_err(|source| RegistryError::BadAddress {
                item: key.to_string(),
                field: "mint",
                source,
            })?;
        let pool = item
            .pool
            .as_deref()
            .unwrap_or_default()
            .parse()
            .map_err(|source| RegistryError::BadAddress {
                item: key.to_string(),
                field: "pool",
                source,
            })?;
        Ok(Market {
            key: key.to_string(),
            item: item.clone(),
            mint,
            pool,
        })
    }

    /// Look up an item by registry key, symbol, mint address or display name
    /// (case-insensitive), in that order of preference.
    pub fn find(&self, needle: &str) -> Option<(&str, &Item)> {
        if let Some((key, item)) = self.items.get_key_value(needle) {
            return Some((key.as_str(), item));
        }
        let lower = needle.to_ascii_lowercase();
        self.items
            .iter()
            .find(|(key, item)| {
                key.eq_ignore_ascii_case(needle)
                    || item.symbol.eq_ignore_ascii_case(needle)
                    || item.mint.as_deref() == Some(needle)
                    || item.name.to_ascii_lowercase() == lower
            })
            .map(|(key, item)| (key.as_str(), item))
    }

    /// Resolve a user-supplied item reference into a tradeable [`Market`].
    pub fn market(&self, needle: &str) -> Result<Market, RegistryError> {
        let (key, item) = self
            .find(needle)
            .ok_or_else(|| RegistryError::UnknownItem(needle.to_string()))?;
        if !item.is_listed() {
            return Err(RegistryError::NotListed(key.to_string()));
        }
        self.resolve_market(key, item)
    }

    /// How many registry entries are live on chain.
    pub fn listed_count(&self) -> usize {
        self.items.values().filter(|i| i.is_listed()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/registry.sample.json");

    fn registry() -> Registry {
        Registry::from_json(SAMPLE).expect("sample registry parses")
    }

    #[test]
    fn parses_and_resolves_markets() {
        let reg = registry();
        assert_eq!(reg.network, "mainnet-beta");
        assert_eq!(reg.items.len(), 3);
        assert_eq!(reg.listed_count(), 2);

        let markets = reg.markets().unwrap();
        assert_eq!(markets.len(), 2, "the unlisted item is excluded");
        let lobster = markets.iter().find(|m| m.key == "lobster").unwrap();
        assert_eq!(lobster.name(), "Lobster");
        assert_eq!(
            lobster.pool.to_string(),
            "76T7nfCqbUcJH9KaA2NcCa5W7RzR5mBj3F44AbjZDLtX"
        );
    }

    #[test]
    fn floor_uses_registry_seed_then_alch_formula() {
        let reg = registry();
        // p0GpPerItem present.
        assert!((reg.items["lobster"].floor_gp() - 54.0).abs() < 1e-9);
        // p0GpPerItem absent -> 0.9 * lowalch.
        let mut no_seed = reg.items["bronze_dagger"].clone();
        no_seed.p0_gp_per_item = None;
        assert!((no_seed.floor_gp() - 3.6).abs() < 1e-9);
    }

    #[test]
    fn find_accepts_key_symbol_name_and_mint() {
        let reg = registry();
        for needle in [
            "lobster",
            "LOBSTER",
            "LOBSTER",
            "Lobster",
            "14RTcWXNRnjiDw5piXPC7zQ2NbFE9VyStDSfP53qRSGP",
        ] {
            assert_eq!(
                reg.find(needle).map(|(k, _)| k),
                Some("lobster"),
                "{needle}"
            );
        }
        assert!(reg.find("dragon_claws").is_none());
    }

    #[test]
    fn unlisted_items_are_rejected_with_a_clear_error() {
        let reg = registry();
        let err = reg.market("shark").unwrap_err();
        assert!(
            matches!(err, RegistryError::NotListed(ref k) if k == "shark"),
            "{err}"
        );
        let err = reg.market("nonsense").unwrap_err();
        assert!(matches!(err, RegistryError::UnknownItem(_)), "{err}");
    }
}

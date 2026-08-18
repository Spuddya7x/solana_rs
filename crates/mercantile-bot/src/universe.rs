//! Choosing which markets to trade.
//!
//! The registry lists well over a thousand items. Most are dust — a pool whose
//! floor is a couple of GP cannot pay for its own fees — so the bot picks a
//! working subset, either explicitly by name or by filtering the registry.

use anyhow::Result;
use mercantile_core::{Market, Registry};

use crate::config::UniverseConfig;

/// Resolve the configured universe against a registry.
///
/// An explicit `include` list wins and is returned in the order given, so an
/// operator naming five items gets exactly those five. Otherwise the filters run
/// and the result is sorted by floor price, descending — the richer markets first,
/// since they are the ones where a trade can clear the fee.
pub fn select(registry: &Registry, config: &UniverseConfig) -> Result<Vec<Market>> {
    if !config.include.is_empty() {
        let mut markets = Vec::with_capacity(config.include.len());
        for key in &config.include {
            if config.exclude.iter().any(|e| e == key) {
                continue;
            }
            markets.push(registry.market(key)?);
        }
        return Ok(markets);
    }

    let mut markets: Vec<Market> = registry
        .markets()?
        .into_iter()
        .filter(|market| {
            let floor = market.floor_gp();
            if floor < config.min_floor_gp || floor > config.max_floor_gp {
                return false;
            }
            if let Some(members_only) = config.members_only {
                if market.item.members != members_only {
                    return false;
                }
            }
            !config.exclude.contains(&market.key)
        })
        .collect();

    markets.sort_by(|a, b| {
        b.floor_gp()
            .partial_cmp(&a.floor_gp())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });
    markets.truncate(config.limit);
    Ok(markets)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version": 1,
      "network": "mainnet-beta",
      "gp": { "mint": "123B7bdJzDYGkrAg7i3JUi5TaHYP47dqmSiR5qPRSGP", "decimals": 6 },
      "items": {
        "lobster":  { "objId": 379,  "name": "Lobster",  "cost": 150, "lowalch": 60, "p0GpPerItem": 54,
          "members": false, "mint": "14RTcWXNRnjiDw5piXPC7zQ2NbFE9VyStDSfP53qRSGP", "pool": "76T7nfCqbUcJH9KaA2NcCa5W7RzR5mBj3F44AbjZDLtX" },
        "shark":    { "objId": 385,  "name": "Shark",    "cost": 300, "lowalch": 120, "p0GpPerItem": 108,
          "members": true,  "mint": "EXTfvP4TLRmsqaWshFeYEBQZR72PACzAqqdmfJyWRSGP", "pool": "HUsZyVsEKcfp51mYyuNSFeNE3VC5zRfrRzCEsKsWTeJu" },
        "bronze_dagger": { "objId": 1205, "name": "Bronze dagger", "cost": 10, "lowalch": 4, "p0GpPerItem": 3.6,
          "members": false, "mint": "5CvmBaSDMeR1stqnv1JM7WXwXHgXhpawT16JVmivRSGP", "pool": "CTsCsfA7CKHx3oTpqAYYDcfcA9PXbDahYH9jS3ayEN6w" },
        "unlisted": { "objId": 1, "name": "Unlisted", "cost": 1000, "lowalch": 400, "members": false, "mint": null, "pool": null }
      }
    }"#;

    fn registry() -> Registry {
        Registry::from_json(SAMPLE).unwrap()
    }

    #[test]
    fn an_explicit_list_is_honoured_in_order() {
        let markets = select(
            &registry(),
            &UniverseConfig {
                include: vec!["shark".to_string(), "lobster".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            markets.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["shark", "lobster"]
        );
    }

    #[test]
    fn an_explicit_list_naming_an_unlisted_item_fails_loudly() {
        let err = select(
            &registry(),
            &UniverseConfig {
                include: vec!["unlisted".to_string()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("unlisted"), "{err}");
    }

    #[test]
    fn filtering_drops_dust_and_sorts_by_floor() {
        let markets = select(
            &registry(),
            &UniverseConfig {
                min_floor_gp: 5.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            markets.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["shark", "lobster"],
            "the 3.6 GP dagger is below the floor threshold"
        );
    }

    #[test]
    fn exclusions_and_limits_apply() {
        let markets = select(
            &registry(),
            &UniverseConfig {
                min_floor_gp: 0.0,
                exclude: vec!["shark".to_string()],
                limit: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(markets.len(), 1);
        assert_eq!(markets[0].key, "lobster");
    }

    #[test]
    fn membership_can_be_filtered_either_way() {
        let members = select(
            &registry(),
            &UniverseConfig {
                members_only: Some(true),
                min_floor_gp: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            members.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["shark"]
        );

        let free = select(
            &registry(),
            &UniverseConfig {
                members_only: Some(false),
                min_floor_gp: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(free.len(), 2);
    }
}

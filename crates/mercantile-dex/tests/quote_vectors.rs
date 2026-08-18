//! Golden-vector test: our quote math against the reference `@meteora-ag/cp-amm-sdk`.
//!
//! `tests/fixtures/quote_vectors.json` holds four live mainnet Mercantile pools
//! (raw account data, base64) and the quotes the JavaScript SDK produced for them
//! at a fixed point in time. Reproducing those numbers exactly — including which
//! trades the SDK refuses — is what makes it safe to size real orders off our own
//! quotes rather than round-tripping through the SDK.

use base64::Engine;
use mercantile_dex::pool::PoolState;
use mercantile_dex::quote::{Side, SwapQuote};
use mercantile_dex::DexError;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Vectors {
    #[serde(rename = "currentPoint")]
    current_point: u64,
    pools: Vec<PoolVector>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct PoolVector {
    key: String,
    data: String,
    #[serde(rename = "spotPrice")]
    spot_price: f64,
}

#[derive(Deserialize)]
struct Case {
    pool: String,
    side: String,
    mode: String,
    amount: String,
    #[serde(rename = "amountIn")]
    amount_in: Option<String>,
    #[serde(rename = "amountOut")]
    amount_out: Option<String>,
    error: Option<String>,
}

fn load() -> (Vectors, HashMap<String, PoolState>) {
    let vectors: Vectors =
        serde_json::from_str(include_str!("fixtures/quote_vectors.json")).expect("vectors parse");
    let states = vectors
        .pools
        .iter()
        .map(|p| {
            let data = base64::engine::general_purpose::STANDARD
                .decode(&p.data)
                .expect("base64");
            (
                p.key.clone(),
                PoolState::decode(&data).expect("decode pool"),
            )
        })
        .collect();
    (vectors, states)
}

#[test]
fn decodes_live_pools() {
    let (vectors, states) = load();
    assert_eq!(states.len(), 4);
    for pool in &vectors.pools {
        let state = &states[&pool.key];
        assert_eq!(
            state.token_b_mint.to_string(),
            "123B7bdJzDYGkrAg7i3JUi5TaHYP47dqmSiR5qPRSGP",
            "{} must be quoted in GP",
            pool.key
        );
        assert_eq!(
            state.collect_fee_mode,
            mercantile_dex::CollectFeeMode::OnlyB
        );
        assert_eq!(state.sqrt_max_price, mercantile_dex::math::MAX_SQRT_PRICE);
        assert_eq!(state.activation_type, 1, "activated by timestamp");
        let spot = state.spot_price();
        assert!(
            (spot - pool.spot_price).abs() / pool.spot_price < 1e-12,
            "{}: {spot} vs {}",
            pool.key,
            pool.spot_price
        );
        // Every pool trades at or above its permanent bid floor, by construction.
        assert!(state.premium_over_floor() >= 1.0, "{}", pool.key);
    }
}

#[test]
fn quotes_match_the_reference_sdk() {
    let (vectors, states) = load();
    let mut checked = 0;
    let mut refusals = 0;

    for case in &vectors.cases {
        let state = &states[&case.pool];
        let side = match case.side.as_str() {
            "buy" => Side::Buy,
            "sell" => Side::Sell,
            other => panic!("unknown side {other}"),
        };
        let amount: u64 = case.amount.parse().unwrap();
        let label = format!("{} {} {} {}", case.pool, case.side, case.mode, case.amount);

        let quoted = match case.mode.as_str() {
            "exact_in" => state.quote_exact_in(side, amount, vectors.current_point),
            "exact_out" => state.quote_exact_out(side, amount, vectors.current_point),
            other => panic!("unknown mode {other}"),
        };

        match (&case.error, quoted) {
            (Some(_), Err(_)) => {
                // Both refuse. The message differs (ours names the pool's range);
                // agreeing on *which* trades are impossible is the property that matters.
                refusals += 1;
            }
            (Some(err), Ok(quote)) => panic!(
                "{label}: SDK refused ({err}) but we quoted in={} out={}",
                quote.amount_in, quote.amount_out
            ),
            (None, Err(err)) => panic!("{label}: SDK quoted but we refused: {err}"),
            (None, Ok(quote)) => {
                let expected_in: u64 = case.amount_in.as_ref().unwrap().parse().unwrap();
                let expected_out: u64 = case.amount_out.as_ref().unwrap().parse().unwrap();
                assert_eq!(quote.amount_in, expected_in, "{label}: amount_in");
                assert_eq!(quote.amount_out, expected_out, "{label}: amount_out");
                checked += 1;
            }
        }
    }

    assert!(
        checked >= 30,
        "expected a broad set of agreeing quotes, got {checked}"
    );
    assert!(
        refusals >= 8,
        "expected the impossible trades to be exercised too, got {refusals}"
    );
}

#[test]
fn refusals_are_specific_about_why() {
    let (vectors, states) = load();
    let lobster = &states["lobster"];

    // More items than the pool holds: the curve runs out before the price does.
    let err = lobster
        .quote_buy_items(100, vectors.current_point)
        .unwrap_err();
    assert!(matches!(err, DexError::InsufficientLiquidity), "{err}");

    // Dumping items pushes the price under the pool's floor, which the range forbids.
    let err = lobster
        .quote_sell_items(25, vectors.current_point)
        .unwrap_err();
    assert!(matches!(err, DexError::PriceRangeViolation { .. }), "{err}");

    // A buy too small to round to a single item base unit.
    let err = lobster
        .quote_exact_in(Side::Buy, 1_000_000, vectors.current_point)
        .unwrap_err();
    assert!(matches!(err, DexError::ZeroAmount), "{err}");
}

#[test]
fn buying_then_selling_loses_the_spread_not_the_bank() {
    let (vectors, states) = load();
    let point = vectors.current_point;
    for (key, state) in &states {
        let buy = state.quote_buy_items(5, point).expect("buy 5");
        let sell = state.quote_sell_items(5, point).expect("sell 5");
        // A round trip at the same instant always costs money: 1% each way plus impact.
        assert!(
            sell.amount_out < buy.amount_in,
            "{key}: round trip must not be free"
        );
        let round_trip_cost = 1.0 - sell.amount_out as f64 / buy.amount_in as f64;
        assert!(
            (0.01..0.30).contains(&round_trip_cost),
            "{key}: round trip cost {round_trip_cost} outside the plausible band"
        );
        // Buying pushes the price up, selling pushes it down.
        assert!(buy.next_sqrt_price > state.sqrt_price, "{key}");
        assert!(sell.next_sqrt_price < state.sqrt_price, "{key}");
    }
}

#[test]
fn quotes_report_usable_economics() {
    let (vectors, states) = load();
    let shark = &states["shark"];
    let quote = shark.quote_buy_items(10, vectors.current_point).unwrap();
    assert_eq!(quote.side, Side::Buy);
    assert!((quote.items() - 10.0).abs() < f64::EPSILON);
    // Paying above spot on a buy, and the impact is the gap between the two.
    assert!(quote.execution_price > quote.spot_price);
    assert!(quote.price_impact_pct > 0.0 && quote.price_impact_pct < 100.0);
    // Slippage guards bracket the quote.
    assert!(quote.max_amount_in(1.0) > quote.amount_in);
    assert!(quote.min_amount_out(1.0) < quote.amount_out);
}

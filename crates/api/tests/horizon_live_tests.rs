//! Live Stellar testnet tests for friendbot funding + balance reads.
//!
//! These hit the real network and are **off by default**. Enable with both:
//!   `DATABASE_URL=...` (Postgres) and `OCTO_LIVE_TESTS=1`
//! e.g. `OCTO_LIVE_TESTS=1 cargo test -p octo-api --test horizon_live_tests`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use octo_api::{build_router, AppState};
use octo_store::Store;
use octo_wallet_core::StellarNetwork;
use std::sync::Once;
use tower::ServiceExt;

static LOAD_ENV: Once = Once::new();

fn enabled() -> bool {
    LOAD_ENV.call_once(|| {
        let _ = dotenvy::dotenv();
    });
    std::env::var("OCTO_LIVE_TESTS").as_deref() == Ok("1") && std::env::var("DATABASE_URL").is_ok()
}

async fn live_state() -> Option<AppState> {
    if !enabled() {
        return None;
    }
    let url = std::env::var("DATABASE_URL").ok()?;
    let store = Store::connect(&url).await.expect("connect");
    store.migrate().await.expect("migrate");
    Some(AppState::new(
        store,
        [7u8; 32],
        StellarNetwork::Testnet,
        "https://horizon-testnet.stellar.org".into(),
        Some("https://friendbot.stellar.org".into()),
    ))
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn create_wallet_funds_and_has_balance() {
    let Some(state) = live_state().await else {
        eprintln!("SKIPPED: set OCTO_LIVE_TESTS=1 and DATABASE_URL to run live testnet tests");
        return;
    };
    let app = build_router(state);

    // Create a wallet — should friendbot-fund on testnet.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/wallets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let wallet = body_json(resp).await;
    let id = wallet["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        wallet["data"]["funded"], true,
        "testnet wallet should be friendbot-funded"
    );

    // Balances should now include a positive native XLM balance.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/wallets/{id}/balances"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bal = body_json(resp).await;
    let balances = bal["data"].as_array().unwrap();
    assert!(!balances.is_empty(), "funded account must have balances");
    let native = balances
        .iter()
        .find(|b| b["asset_type"] == "native")
        .expect("native balance present");
    let amount: f64 = native["balance"].as_str().unwrap().parse().unwrap();
    assert!(
        amount > 0.0,
        "native balance must be positive after funding"
    );
}

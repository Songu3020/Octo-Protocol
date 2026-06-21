//! Integration tests for the octo API. Require Postgres via `DATABASE_URL` (loaded from .env).
//!
//! These drive the real axum router with in-process requests, exercising
//! crypto + wallet-core + store together. Skipped (with a message) if no DATABASE_URL.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use octo_api::{build_router, AppState};
use octo_store::Store;
use octo_wallet_core::StellarNetwork;
use std::sync::Once;
use tower::ServiceExt; // for `oneshot`

static LOAD_ENV: Once = Once::new();

fn database_url() -> Option<String> {
    LOAD_ENV.call_once(|| {
        let _ = dotenvy::dotenv();
    });
    std::env::var("DATABASE_URL").ok()
}

async fn test_state() -> Option<AppState> {
    let url = database_url()?;
    let store = Store::connect(&url).await.expect("connect");
    store.migrate().await.expect("migrate");
    let master_key = [42u8; 32]; // deterministic test key
    Some(AppState::new(
        store,
        master_key,
        StellarNetwork::Testnet,
        "https://horizon-testnet.stellar.org".into(),
        None,
    ))
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("json")
}

fn post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

/// GET with an Authorization bearer token.
fn get_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

/// POST with no body but an Authorization bearer token.
fn post_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

/// Sign up a fresh user via the router and return its bearer token.
async fn auth_token(app: &axum::Router) -> String {
    let email = format!("u-{}@octo.test", uuid::Uuid::new_v4().simple());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/signup")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"email":"{email}","password":"supersecret"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    body_json(resp).await["data"]["token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn create_wallet_returns_account_and_mnemonic() {
    let Some(state) = test_state().await else {
        eprintln!("SKIPPED: set DATABASE_URL (start `docker compose up -d db`)");
        return;
    };
    let app = build_router(state.clone());
    let token = auth_token(&app).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/wallets")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"label":"acme"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    let data = &json["data"];
    assert!(
        data["address"].as_str().unwrap().starts_with('G'),
        "master address must be a G... account"
    );
    assert_eq!(
        data["recovery_mnemonic"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .count(),
        12,
        "a 12-word recovery mnemonic must be returned once"
    );
    assert_eq!(data["network"], "testnet");

    // The stored seed must be ciphertext — never the plaintext mnemonic.
    let wallet_id = data["id"].as_str().unwrap();
    let row: (Vec<u8>,) =
        sqlx::query_as("SELECT sealed_ciphertext FROM wallets WHERE id = $1::uuid")
            .bind(wallet_id)
            .fetch_one(state.store().pool())
            .await
            .unwrap();
    assert!(!row.0.is_empty(), "sealed ciphertext stored");
    let mnemonic = data["recovery_mnemonic"].as_str().unwrap();
    let needle = &mnemonic.as_bytes()[..mnemonic.len().min(12)];
    assert!(
        !row.0.windows(needle.len()).any(|w| w == needle),
        "plaintext mnemonic must not appear in stored ciphertext"
    );
}

#[tokio::test]
async fn addresses_return_both_forms_and_share_base() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;

    // Create a wallet (empty body is allowed).
    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();
    let wallet = body_json(resp).await;
    let wallet_id = wallet["data"]["id"].as_str().unwrap().to_string();
    let base = wallet["data"]["address"].as_str().unwrap().to_string();

    // Create two addresses.
    let mut muxed = vec![];
    let mut memo_ids = vec![];
    for _ in 0..2 {
        let uri = format!("/v1/wallets/{wallet_id}/addresses");
        let resp = app.clone().oneshot(post_auth(&uri, &token)).await.unwrap();
        let st = resp.status();
        let j = body_json(resp).await;
        assert_eq!(st, StatusCode::CREATED, "address create failed: {j}");
        let d = &j["data"];
        assert!(d["muxed_address"].as_str().unwrap().starts_with('M'));
        // The fallback form shares the same base G... account.
        assert_eq!(d["base_address"].as_str().unwrap(), base);
        muxed.push(d["muxed_address"].as_str().unwrap().to_string());
        memo_ids.push(d["memo_id"].as_i64().unwrap());
    }

    assert_ne!(muxed[0], muxed[1], "distinct muxed addresses");
    assert_eq!(memo_ids, vec![1, 2], "ids allocated sequentially from 1");

    // List returns both.
    let uri = format!("/v1/wallets/{wallet_id}/addresses");
    let resp = app.oneshot(get_auth(&uri, &token)).await.unwrap();
    let list = body_json(resp).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn transactions_endpoint_returns_list() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;

    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();
    let wallet_id = body_json(resp).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // A new wallet has no transactions yet → empty array, 200.
    let uri = format!("/v1/wallets/{wallet_id}/transactions");
    let resp = app.clone().oneshot(get_auth(&uri, &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp).await;
    assert_eq!(j["data"].as_array().unwrap().len(), 0);

    // Unknown wallet (authed user) → 404.
    let uri = format!("/v1/wallets/{}/transactions", uuid::Uuid::new_v4());
    let resp = app.oneshot(get_auth(&uri, &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_unknown_wallet_is_404() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;
    let uri = format!("/v1/wallets/{}", uuid::Uuid::new_v4());
    let resp = app.oneshot(get_auth(&uri, &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unauthenticated_request_is_401() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    // No token at all → 401 (auth required on wallet endpoints).
    let uri = format!("/v1/wallets/{}", uuid::Uuid::new_v4());
    let resp = app.oneshot(get(&uri)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn addresses_on_unknown_wallet_is_404() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;
    let uri = format!("/v1/wallets/{}/addresses", uuid::Uuid::new_v4());
    let resp = app.oneshot(post_auth(&uri, &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn post_json_auth(uri: &str, body: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn withdraw_requires_destination_amount_and_idempotency_key() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;
    // Create a wallet to target.
    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();
    let wallet_id = body_json(resp).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let uri = format!("/v1/wallets/{wallet_id}/withdraw");

    // Missing everything.
    let resp = app
        .clone()
        .oneshot(post_json_auth(&uri, "{}", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Missing idempotency key (has dest + amount).
    let body = r#"{"destination":"GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6","amount_stroops":100}"#;
    let resp = app
        .oneshot(post_json_auth(&uri, body, &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn withdraw_duplicate_idempotency_key_conflicts_before_signing() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state.clone());
    let token = auth_token(&app).await;
    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();
    let wallet_id = body_json(resp).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let uri = format!("/v1/wallets/{wallet_id}/withdraw");

    // Pre-insert a withdrawal with a known idempotency key (simulating a prior request) so the
    // second attempt conflicts at create_withdrawal BEFORE any Horizon/signing happens.
    let key = format!("key-{}", uuid::Uuid::new_v4());
    state
        .store()
        .create_withdrawal(octo_store::NewWithdrawal {
            wallet_id: wallet_id.parse().unwrap(),
            idempotency_key: &key,
            destination_account: "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6",
            asset_code: "native",
            asset_issuer: None,
            amount_stroops: 100,
            memo_id: None,
        })
        .await
        .unwrap();

    let body = format!(
        r#"{{"destination":"GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6","amount_stroops":100,"idempotency_key":"{key}"}}"#
    );
    let resp = app
        .oneshot(post_json_auth(&uri, &body, &token))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "retry with same idempotency key must 409 (no double-spend)"
    );
}

#[tokio::test]
async fn api_key_generate_and_get() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;

    // Create a wallet owned by this user.
    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();
    let wallet_id = body_json(resp).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Before generation: not configured.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/wallets/{wallet_id}/api-key"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_json(resp).await["data"]["configured"], false);

    // Generate → returns the full key once, prefixed octo_sk_test_.
    let resp = app
        .clone()
        .oneshot(post_auth(
            &format!("/v1/wallets/{wallet_id}/api-key"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let j = body_json(resp).await;
    let key = j["data"]["api_key"].as_str().unwrap().to_string();
    assert!(key.starts_with("octo_sk_test_"), "key was {key}");
    assert!(key.len() > 20);

    // Get → configured, prefix only (never the full key).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/wallets/{wallet_id}/api-key"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let j = body_json(resp).await;
    assert_eq!(j["data"]["configured"], true);
    let prefix = j["data"]["prefix"].as_str().unwrap();
    assert!(key.starts_with(prefix), "prefix must match the key");
    assert!(
        prefix.len() < key.len(),
        "prefix must be shorter than the key"
    );
}

#[tokio::test]
async fn api_key_requires_ownership() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);

    // User A creates a wallet.
    let token_a = auth_token(&app).await;
    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token_a))
        .await
        .unwrap();
    let wallet_id = body_json(resp).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // User B cannot generate a key for A's wallet → 404 (not revealed).
    let token_b = auth_token(&app).await;
    let resp = app
        .oneshot(post_auth(
            &format!("/v1/wallets/{wallet_id}/api-key"),
            &token_b,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Generate an API key for a wallet and return the full key string.
async fn api_key_for(app: &axum::Router, token: &str, wallet_id: &str) -> String {
    let resp = app
        .clone()
        .oneshot(post_auth(
            &format!("/v1/wallets/{wallet_id}/api-key"),
            token,
        ))
        .await
        .unwrap();
    body_json(resp).await["data"]["api_key"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn api_key_can_create_address_on_its_wallet() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;

    // Create a wallet + its API key.
    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();
    let wallet_id = body_json(resp).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let key = api_key_for(&app, &token, &wallet_id).await;
    assert!(key.starts_with("octo_sk_"));

    // Use the API KEY (not the login token) to create a deposit address.
    let resp = app
        .oneshot(post_auth(
            &format!("/v1/wallets/{wallet_id}/addresses"),
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "API key should create addresses"
    );
    let j = body_json(resp).await;
    assert!(j["data"]["muxed_address"]
        .as_str()
        .unwrap()
        .starts_with('M'));
}

#[tokio::test]
async fn api_key_cannot_touch_another_wallet() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;

    // Two wallets owned by the same user; key for wallet A.
    let a = body_json(
        app.clone()
            .oneshot(post_auth("/v1/wallets", &token))
            .await
            .unwrap(),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let b = body_json(
        app.clone()
            .oneshot(post_auth("/v1/wallets", &token))
            .await
            .unwrap(),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let key_a = api_key_for(&app, &token, &a).await;

    // Key A on wallet B → 404 (scope enforced, existence not revealed).
    let resp = app
        .oneshot(post_auth(&format!("/v1/wallets/{b}/addresses"), &key_a))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_key_cannot_withdraw() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;

    let wallet_id = body_json(
        app.clone()
            .oneshot(post_auth("/v1/wallets", &token))
            .await
            .unwrap(),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let key = api_key_for(&app, &token, &wallet_id).await;

    // Withdrawals are dashboard-only: an API key is rejected with 401.
    let body = r#"{"destination":"GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6","amount_stroops":100,"idempotency_key":"k1"}"#;
    let resp = app
        .oneshot(post_json_auth(
            &format!("/v1/wallets/{wallet_id}/withdraw"),
            body,
            &key,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "API keys must not be allowed to withdraw"
    );
}

#[tokio::test]
async fn audit_logs_record_and_list() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);

    // Signup records "created an account"; capture the token.
    let email = format!("audit-{}@octo.test", uuid::Uuid::new_v4().simple());
    let resp = app
        .clone()
        .oneshot(post_json(
            "/v1/auth/signup",
            &format!(r#"{{"email":"{email}","password":"supersecret"}}"#),
        ))
        .await
        .unwrap();
    let token = body_json(resp).await["data"]["token"]
        .as_str()
        .unwrap()
        .to_string();

    // Create a wallet → records "created master wallet".
    app.clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();

    // List all audit logs for this user.
    let resp = app
        .clone()
        .oneshot(get_auth("/v1/audit-logs", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let logs = body_json(resp).await;
    let arr = logs["data"].as_array().unwrap();
    assert!(
        arr.len() >= 2,
        "expected signup + wallet events, got {}",
        arr.len()
    );
    let actions: Vec<&str> = arr.iter().map(|l| l["action"].as_str().unwrap()).collect();
    assert!(actions.iter().any(|a| a.contains("account")));
    assert!(actions.iter().any(|a| a.contains("wallet")));

    // Filter by category=wallet → only wallet events.
    let resp = app
        .oneshot(get_auth("/v1/audit-logs?category=wallet", &token))
        .await
        .unwrap();
    let filtered = body_json(resp).await;
    let arr = filtered["data"].as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr.iter().all(|l| l["category"] == "wallet"));
}

#[tokio::test]
async fn sponsor_returns_429_when_budget_exhausted() {
    let Some(state) = test_state().await else {
        return;
    };
    let app = build_router(state);
    let token = auth_token(&app).await;
    let resp = app
        .clone()
        .oneshot(post_auth("/v1/wallets", &token))
        .await
        .unwrap();
    let wallet_id = body_json(resp).await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let uri = format!("/v1/wallets/{wallet_id}/sponsor");

    // Budget of 100 stroops permits exactly one 100-stroop sponsorship.
    let mk_body = || {
        format!(
            r#"{{"inner_tx_hash":"{}","fee_stroops":100,"daily_budget_stroops":100}}"#,
            uuid::Uuid::new_v4()
        )
    };

    let first = app
        .clone()
        .oneshot(post_json_auth(&uri, &mk_body(), &token))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED, "first request fits");

    // A second, distinct request with no budget left must 429, not silently overspend.
    let second = app
        .oneshot(post_json_auth(&uri, &mk_body(), &token))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = body_json(second).await;
    assert_eq!(body["message"], "budget_exceeded");
}

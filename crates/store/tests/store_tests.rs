//! Integration tests for octo-store. Require a running Postgres.
//!
//! Run with: `docker compose up -d db` then `cargo test -p octo-store`.
//!
//! `DATABASE_URL` is read from the workspace `.env` automatically (via dotenvy), so the plain
//! `cargo test -p octo-store` works without exporting anything. If no URL can be found, the tests
//! print a clear SKIPPED message and pass (so a DB-less `cargo test` of the whole workspace is
//! green). If a URL is found but the DB is unreachable, the test fails loudly with the reason.

use octo_store::{NewDeposit, NewWallet, NewWithdrawal, Store, StoreError};
use std::sync::Once;
use uuid::Uuid;

static LOAD_ENV: Once = Once::new();

/// Resolve `DATABASE_URL`, loading the workspace `.env` first. Returns `None` only if no URL is
/// configured anywhere (in which case tests skip with a message).
fn database_url() -> Option<String> {
    LOAD_ENV.call_once(|| {
        // Search upward from the crate dir for a .env (workspace root holds it).
        let _ = dotenvy::dotenv();
    });
    std::env::var("DATABASE_URL").ok()
}

async fn store() -> Option<Store> {
    let Some(url) = database_url() else {
        eprintln!(
            "SKIPPED: DATABASE_URL is not set (no .env found). \
             Run `docker compose up -d db` and ensure .env exists to run store tests."
        );
        return None;
    };
    let store = Store::connect(&url)
        .await
        .unwrap_or_else(|e| panic!("could not connect to {url}: {e}"));
    store.migrate().await.expect("migrate");
    Some(store)
}

/// Create a throwaway wallet with a unique account id (so tests don't collide).
async fn fresh_wallet(store: &Store) -> Uuid {
    let acct = format!("G{}", Uuid::new_v4().simple()); // unique, not a real strkey (fine for store tests)
    let w = store
        .create_wallet(NewWallet {
            network: "testnet",
            stellar_account_g: &acct,
            sealed_ciphertext: b"ciphertext",
            sealed_nonce: b"nonce12bytes",
            sealed_salt: b"saltsaltsaltsalt",
            label: Some("test"),
            user_id: None,
            description: None,
        })
        .await
        .expect("create wallet");
    w.id
}

#[tokio::test]
async fn create_and_get_wallet() {
    let Some(store) = store().await else { return };
    let id = fresh_wallet(&store).await;
    let w = store.get_wallet(id).await.expect("get");
    assert_eq!(w.network, "testnet");
    assert_eq!(w.next_muxed_id, 1);
}

#[tokio::test]
async fn allocate_address_increments_atomically() {
    let Some(store) = store().await else { return };
    let wallet_id = fresh_wallet(&store).await;

    // muxed_address is globally unique in the schema (real ones encode the base account), so make
    // the test value unique per wallet too.
    let wid = wallet_id.simple();
    let a = store
        .allocate_address(
            wallet_id,
            |id| Ok(format!("M{wid}-{id}")),
            Some("user-a"),
            serde_json::json!({}),
        )
        .await
        .expect("alloc a");
    let b = store
        .allocate_address(
            wallet_id,
            |id| Ok(format!("M{wid}-{id}")),
            Some("user-b"),
            serde_json::json!({}),
        )
        .await
        .expect("alloc b");

    assert_eq!(a.muxed_id, 1);
    assert_eq!(b.muxed_id, 2);
    assert_ne!(a.muxed_address, b.muxed_address);

    let list = store.list_addresses(wallet_id).await.expect("list");
    assert_eq!(list.len(), 2);
}

#[tokio::test]
async fn record_deposit_is_idempotent() {
    let Some(store) = store().await else { return };
    let wallet_id = fresh_wallet(&store).await;
    let tx_hash = Uuid::new_v4().to_string();

    let dep = NewDeposit {
        wallet_id,
        address_id: None,
        asset_code: "native".into(),
        asset_issuer: None,
        amount_stroops: 10_000_000,
        source_account: Some("Gsender".into()),
        destination_account: Some("Gmaster".into()),
        stellar_tx_hash: tx_hash.clone(),
        operation_index: 0,
        horizon_op_id: format!("{tx_hash}-0"),
        ledger: Some(123),
        memo_id: None,
    };

    // First insert credits.
    let first = store.record_deposit(&dep).await.expect("first");
    assert!(first.is_some(), "first deposit must be recorded");

    // Replaying the SAME horizon_op_id must NOT double-credit.
    let second = store.record_deposit(&dep).await.expect("second");
    assert!(
        second.is_none(),
        "duplicate deposit must be a no-op (anti double-credit)"
    );

    let txs = store.list_transactions(wallet_id).await.expect("list");
    assert_eq!(txs.len(), 1, "exactly one ledger entry for one on-chain op");
}

#[tokio::test]
async fn different_op_index_same_tx_is_distinct() {
    let Some(store) = store().await else { return };
    let wallet_id = fresh_wallet(&store).await;
    let tx_hash = Uuid::new_v4().to_string();

    let base = NewDeposit {
        wallet_id,
        address_id: None,
        asset_code: "native".into(),
        asset_issuer: None,
        amount_stroops: 5,
        source_account: None,
        destination_account: None,
        stellar_tx_hash: tx_hash.clone(),
        operation_index: 0,
        horizon_op_id: format!("{tx_hash}-0"),
        ledger: None,
        memo_id: None,
    };
    let op1 = NewDeposit {
        operation_index: 1,
        horizon_op_id: format!("{tx_hash}-1"),
        ..base.clone()
    };

    assert!(store.record_deposit(&base).await.expect("op0").is_some());
    assert!(store.record_deposit(&op1).await.expect("op1").is_some());
    assert_eq!(store.list_transactions(wallet_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn withdrawal_idempotency_key_blocks_double_spend() {
    let Some(store) = store().await else { return };
    let wallet_id = fresh_wallet(&store).await;

    let mk = |key: &'static str| NewWithdrawal {
        wallet_id,
        idempotency_key: key,
        destination_account: "Gdest",
        asset_code: "native",
        asset_issuer: None,
        amount_stroops: 1_000,
        memo_id: None,
    };

    let first = store.create_withdrawal(mk("key-1")).await;
    assert!(first.is_ok(), "first withdrawal accepted");

    // Same idempotency key => conflict, not a second payout.
    let second = store.create_withdrawal(mk("key-1")).await;
    assert!(
        matches!(second, Err(StoreError::Conflict)),
        "retry must conflict"
    );

    // A different key is a different withdrawal.
    let third = store.create_withdrawal(mk("key-2")).await;
    assert!(third.is_ok());
}

#[tokio::test]
async fn concurrent_sponsor_requests_cannot_exceed_daily_budget() {
    let Some(store) = store().await else { return };
    let wallet_id = fresh_wallet(&store).await;

    // Budget permits exactly 5 of the 10 requests below (100 stroops each, budget 500).
    const FEE: i64 = 100;
    const BUDGET: i64 = 500;
    const ATTEMPTS: usize = 10;

    let mut tasks = Vec::with_capacity(ATTEMPTS);
    for _ in 0..ATTEMPTS {
        let store = store.clone();
        let inner_tx_hash = Uuid::new_v4().to_string();
        tasks.push(tokio::task::spawn(async move {
            store
                .record_sponsored_tx_if_budget_available(wallet_id, &inner_tx_hash, FEE, BUDGET)
                .await
                .expect("query succeeds")
        }));
    }

    let mut accepted = 0;
    let mut rejected = 0;
    for task in tasks {
        match task.await.expect("task panicked") {
            Some(_) => accepted += 1,
            None => rejected += 1,
        }
    }

    assert_eq!(accepted, 5, "exactly 5 requests fit in the budget");
    assert_eq!(rejected, 5, "the other 5 must be rejected, not overspend");

    let total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(fee_stroops), 0)::BIGINT FROM sponsored_transactions WHERE wallet_id = $1",
    )
    .bind(wallet_id)
    .fetch_one(store.pool())
    .await
    .expect("sum");
    assert_eq!(total, BUDGET, "spend never exceeds the configured budget");
}

#[tokio::test]
async fn cursor_roundtrip() {
    let Some(store) = store().await else { return };
    let wallet_id = fresh_wallet(&store).await;

    assert_eq!(store.get_cursor(wallet_id).await.unwrap(), None);
    store.set_cursor(wallet_id, "token-1").await.unwrap();
    assert_eq!(
        store.get_cursor(wallet_id).await.unwrap().as_deref(),
        Some("token-1")
    );
    // Upsert overwrites.
    store.set_cursor(wallet_id, "token-2").await.unwrap();
    assert_eq!(
        store.get_cursor(wallet_id).await.unwrap().as_deref(),
        Some("token-2")
    );
}

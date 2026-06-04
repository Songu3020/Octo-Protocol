//! Integration tests for octo-store. Require a running Postgres via `DATABASE_URL`.
//!
//! Run with: `docker compose up -d db` then `cargo test -p octo-store`.
//! If `DATABASE_URL` is unset, the tests skip (so a DB-less `cargo test` still passes).

use octo_store::{NewDeposit, NewWallet, NewWithdrawal, Store, StoreError};
use uuid::Uuid;

async fn store() -> Option<Store> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let store = Store::connect(&url).await.expect("connect");
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
        ledger: Some(123),
        memo_id: None,
    };

    // First insert credits.
    let first = store.record_deposit(&dep).await.expect("first");
    assert!(first.is_some(), "first deposit must be recorded");

    // Replaying the SAME (tx_hash, op_index) must NOT double-credit.
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
        ledger: None,
        memo_id: None,
    };
    let op1 = NewDeposit {
        operation_index: 1,
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

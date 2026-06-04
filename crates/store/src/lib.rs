//! Postgres persistence for octo (sqlx).
//!
//! Tables: `wallets`, `addresses`, `transactions`, `withdrawals`, `webhook_endpoints`,
//! `webhook_deliveries`, `ingest_cursor` — see `migrations/0001_init.sql`.
//!
//! Security-relevant guarantees implemented here (see `docs/threat-model.md`):
//! - All queries are parameterized (no string-built SQL) → no SQL injection.
//! - [`Store::allocate_address`] increments the per-wallet muxed-id counter **atomically** inside a
//!   transaction, so concurrent address creation can't collide or reuse an id.
//! - [`Store::record_deposit`] is **idempotent** on the immutable `(tx_hash, operation_index)`
//!   unique index, so a replayed/reorged Horizon event cannot double-credit.
//! - [`Store::create_withdrawal`] is idempotent on `(wallet_id, idempotency_key)`.
#![forbid(unsafe_code)]

mod error;
mod models;

pub use error::StoreError;
pub use models::{Address, NewDeposit, Transaction, Wallet, WebhookEndpoint, Withdrawal};

use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

/// Embedded migrations, applied by [`Store::migrate`].
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// A handle to the database (cloneable; wraps a connection pool).
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

/// Parameters for creating a master wallet.
pub struct NewWallet<'a> {
    pub network: &'a str,
    pub stellar_account_g: &'a str,
    pub sealed_ciphertext: &'a [u8],
    pub sealed_nonce: &'a [u8],
    pub sealed_salt: &'a [u8],
    pub label: Option<&'a str>,
}

/// Parameters for creating a withdrawal intent.
pub struct NewWithdrawal<'a> {
    pub wallet_id: Uuid,
    pub idempotency_key: &'a str,
    pub destination_account: &'a str,
    pub asset_code: &'a str,
    pub asset_issuer: Option<&'a str>,
    pub amount_stroops: i64,
    pub memo_id: Option<i64>,
}

impl Store {
    /// Connect to Postgres at `database_url` and return a pooled handle.
    pub async fn connect(database_url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// Build a store from an existing pool (useful in tests).
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Apply all pending migrations.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    /// Borrow the underlying pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // --- wallets ----------------------------------------------------------

    /// Create a master wallet. Fails with [`StoreError::Conflict`] if the account already exists.
    pub async fn create_wallet(&self, new: NewWallet<'_>) -> Result<Wallet, StoreError> {
        sqlx::query_as::<_, Wallet>(
            r#"
            INSERT INTO wallets
                (network, stellar_account_g, sealed_ciphertext, sealed_nonce, sealed_salt, label)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(new.network)
        .bind(new.stellar_account_g)
        .bind(new.sealed_ciphertext)
        .bind(new.sealed_nonce)
        .bind(new.sealed_salt)
        .bind(new.label)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from_sqlx_conflict)
    }

    /// Fetch a wallet by id.
    pub async fn get_wallet(&self, id: Uuid) -> Result<Wallet, StoreError> {
        sqlx::query_as::<_, Wallet>("SELECT * FROM wallets WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound)
    }

    // --- addresses --------------------------------------------------------

    /// Atomically allocate the next muxed id for `wallet_id` and insert the address row.
    ///
    /// The counter bump and the insert happen in one transaction with a row lock, so two
    /// concurrent callers always get distinct, gap-free-enough ids and never collide.
    pub async fn allocate_address(
        &self,
        wallet_id: Uuid,
        muxed_address_for: impl FnOnce(i64) -> Result<String, ()>,
        customer_ref: Option<&str>,
        metadata: serde_json::Value,
    ) -> Result<Address, StoreError> {
        let mut tx = self.pool.begin().await?;

        // Lock the wallet row and read+bump the counter.
        let next_id: i64 =
            sqlx::query_scalar("SELECT next_muxed_id FROM wallets WHERE id = $1 FOR UPDATE")
                .bind(wallet_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(StoreError::NotFound)?;

        sqlx::query("UPDATE wallets SET next_muxed_id = next_muxed_id + 1, updated_at = now() WHERE id = $1")
            .bind(wallet_id)
            .execute(&mut *tx)
            .await?;

        // Derive the muxed address for this id via the caller-provided closure (wallet-core).
        let muxed_address = muxed_address_for(next_id).map_err(|_| StoreError::NotFound)?;

        let address = sqlx::query_as::<_, Address>(
            r#"
            INSERT INTO addresses (wallet_id, muxed_id, muxed_address, customer_ref, metadata)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(wallet_id)
        .bind(next_id)
        .bind(&muxed_address)
        .bind(customer_ref)
        .bind(metadata)
        .fetch_one(&mut *tx)
        .await
        .map_err(StoreError::from_sqlx_conflict)?;

        tx.commit().await?;
        Ok(address)
    }

    /// List addresses for a wallet (most recent first).
    pub async fn list_addresses(&self, wallet_id: Uuid) -> Result<Vec<Address>, StoreError> {
        let rows = sqlx::query_as::<_, Address>(
            "SELECT * FROM addresses WHERE wallet_id = $1 ORDER BY created_at DESC",
        )
        .bind(wallet_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Find the address for a given `(wallet_id, muxed_id)`, if any.
    pub async fn address_by_muxed_id(
        &self,
        wallet_id: Uuid,
        muxed_id: i64,
    ) -> Result<Option<Address>, StoreError> {
        let row = sqlx::query_as::<_, Address>(
            "SELECT * FROM addresses WHERE wallet_id = $1 AND muxed_id = $2",
        )
        .bind(wallet_id)
        .bind(muxed_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // --- transactions (deposits) ------------------------------------------

    /// Idempotently record a confirmed deposit.
    ///
    /// Returns `Ok(Some(tx))` on first insert and `Ok(None)` if this exact on-chain operation was
    /// already recorded (the `(tx_hash, operation_index)` unique index fired) — so replays and
    /// reorged re-deliveries never double-credit.
    pub async fn record_deposit(&self, d: &NewDeposit) -> Result<Option<Transaction>, StoreError> {
        let result = sqlx::query_as::<_, Transaction>(
            r#"
            INSERT INTO transactions
                (wallet_id, address_id, direction, asset_code, asset_issuer, amount_stroops,
                 source_account, destination_account, stellar_tx_hash, operation_index, ledger,
                 memo_id, status)
            VALUES ($1, $2, 'deposit', $3, $4, $5, $6, $7, $8, $9, $10, $11, 'confirmed')
            RETURNING *
            "#,
        )
        .bind(d.wallet_id)
        .bind(d.address_id)
        .bind(&d.asset_code)
        .bind(&d.asset_issuer)
        .bind(d.amount_stroops)
        .bind(&d.source_account)
        .bind(&d.destination_account)
        .bind(&d.stellar_tx_hash)
        .bind(d.operation_index)
        .bind(d.ledger)
        .bind(d.memo_id)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(tx) => Ok(Some(tx)),
            Err(e) => match StoreError::from_sqlx_conflict(e) {
                StoreError::Conflict => Ok(None), // already recorded — benign
                other => Err(other),
            },
        }
    }

    /// List transactions for a wallet (most recent first).
    pub async fn list_transactions(&self, wallet_id: Uuid) -> Result<Vec<Transaction>, StoreError> {
        let rows = sqlx::query_as::<_, Transaction>(
            "SELECT * FROM transactions WHERE wallet_id = $1 ORDER BY created_at DESC",
        )
        .bind(wallet_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // --- withdrawals ------------------------------------------------------

    /// Create a withdrawal intent. Idempotent on `(wallet_id, idempotency_key)`: a retried request
    /// with the same key returns [`StoreError::Conflict`] instead of creating a second payout.
    pub async fn create_withdrawal(
        &self,
        new: NewWithdrawal<'_>,
    ) -> Result<Withdrawal, StoreError> {
        sqlx::query_as::<_, Withdrawal>(
            r#"
            INSERT INTO withdrawals
                (wallet_id, idempotency_key, destination_account, asset_code, asset_issuer,
                 amount_stroops, memo_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(new.wallet_id)
        .bind(new.idempotency_key)
        .bind(new.destination_account)
        .bind(new.asset_code)
        .bind(new.asset_issuer)
        .bind(new.amount_stroops)
        .bind(new.memo_id)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from_sqlx_conflict)
    }

    // --- ingest cursor ----------------------------------------------------

    /// Read the saved Horizon paging token for a wallet, if any.
    pub async fn get_cursor(&self, wallet_id: Uuid) -> Result<Option<String>, StoreError> {
        let token: Option<String> =
            sqlx::query_scalar("SELECT paging_token FROM ingest_cursor WHERE wallet_id = $1")
                .bind(wallet_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten();
        Ok(token)
    }

    /// Upsert the Horizon paging token for a wallet (durable resume point).
    pub async fn set_cursor(&self, wallet_id: Uuid, paging_token: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO ingest_cursor (wallet_id, paging_token, updated_at)
            VALUES ($1, $2, now())
            ON CONFLICT (wallet_id)
            DO UPDATE SET paging_token = EXCLUDED.paging_token, updated_at = now()
            "#,
        )
        .bind(wallet_id)
        .bind(paging_token)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

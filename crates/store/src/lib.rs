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
pub use models::{
    Address, ApiKey, AuditLog, GasSponsorshipConfig, NewDeposit, NewSponsoredTx,
    SponsoredTransaction, Transaction, User, Wallet, WebhookEndpoint, Withdrawal,
};

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
    pub user_id: Option<Uuid>,
    pub description: Option<&'a str>,
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

    // --- users ------------------------------------------------------------

    /// Create a user. `email` should already be lowercased by the caller. Returns
    /// [`StoreError::Conflict`] if the email is already registered.
    pub async fn create_user(&self, email: &str, password_hash: &str) -> Result<User, StoreError> {
        sqlx::query_as::<_, User>(
            "INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING *",
        )
        .bind(email)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from_sqlx_conflict)
    }

    /// Look up a user by email (caller lowercases).
    pub async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, StoreError> {
        let row = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Fetch a user by id.
    pub async fn get_user(&self, id: Uuid) -> Result<Option<User>, StoreError> {
        let row = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    // --- audit logs -------------------------------------------------------

    /// Append an audit-log entry. Best-effort: failures are surfaced to the caller, which logs and
    /// continues (auditing must never block the primary operation).
    pub async fn record_audit(
        &self,
        user_id: Uuid,
        action: &str,
        category: &str,
        target: Option<&str>,
        ip_address: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO audit_logs (user_id, action, category, target, ip_address)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(action)
        .bind(category)
        .bind(target)
        .bind(ip_address)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List a user's audit logs (most recent first), optionally filtered by `category` and a
    /// case-insensitive `search` over the action/target. Capped at `limit` rows.
    pub async fn list_audit_logs(
        &self,
        user_id: Uuid,
        category: Option<&str>,
        search: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AuditLog>, StoreError> {
        // Build with optional filters; `$2`/`$3` are NULL when not provided.
        let rows = sqlx::query_as::<_, AuditLog>(
            r#"
            SELECT * FROM audit_logs
            WHERE user_id = $1
              AND ($2::text IS NULL OR category = $2)
              AND ($3::text IS NULL OR action ILIKE '%' || $3 || '%'
                                    OR coalesce(target, '') ILIKE '%' || $3 || '%')
            ORDER BY created_at DESC
            LIMIT $4
            "#,
        )
        .bind(user_id)
        .bind(category)
        .bind(search)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // --- api keys ---------------------------------------------------------

    /// Create or replace the wallet's API key (regenerate). Stores only the hash + display prefix.
    pub async fn upsert_api_key(
        &self,
        wallet_id: Uuid,
        prefix: &str,
        key_hash: &str,
    ) -> Result<ApiKey, StoreError> {
        sqlx::query_as::<_, ApiKey>(
            r#"
            INSERT INTO api_keys (wallet_id, prefix, key_hash)
            VALUES ($1, $2, $3)
            ON CONFLICT (wallet_id)
            DO UPDATE SET prefix = EXCLUDED.prefix, key_hash = EXCLUDED.key_hash,
                          created_at = now()
            RETURNING *
            "#,
        )
        .bind(wallet_id)
        .bind(prefix)
        .bind(key_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::Database)
    }

    /// Get the wallet's API key metadata (prefix only — never the secret), if one exists.
    pub async fn get_api_key(&self, wallet_id: Uuid) -> Result<Option<ApiKey>, StoreError> {
        let row = sqlx::query_as::<_, ApiKey>("SELECT * FROM api_keys WHERE wallet_id = $1")
            .bind(wallet_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Look up the wallet that owns a key by its hash (for API-key authentication later).
    pub async fn wallet_id_for_key_hash(&self, key_hash: &str) -> Result<Option<Uuid>, StoreError> {
        let row: Option<(Uuid,)> =
            sqlx::query_as("SELECT wallet_id FROM api_keys WHERE key_hash = $1")
                .bind(key_hash)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    // --- wallets ----------------------------------------------------------

    /// Create a master wallet. Fails with [`StoreError::Conflict`] if the account already exists.
    pub async fn create_wallet(&self, new: NewWallet<'_>) -> Result<Wallet, StoreError> {
        sqlx::query_as::<_, Wallet>(
            r#"
            INSERT INTO wallets
                (network, stellar_account_g, sealed_ciphertext, sealed_nonce, sealed_salt, label,
                 user_id, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(new.network)
        .bind(new.stellar_account_g)
        .bind(new.sealed_ciphertext)
        .bind(new.sealed_nonce)
        .bind(new.sealed_salt)
        .bind(new.label)
        .bind(new.user_id)
        .bind(new.description)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from_sqlx_conflict)
    }

    /// List a user's wallets (most recent first).
    pub async fn list_wallets_for_user(&self, user_id: Uuid) -> Result<Vec<Wallet>, StoreError> {
        let rows = sqlx::query_as::<_, Wallet>(
            "SELECT * FROM wallets WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List all wallets (used by the ingest supervisor to fan out poll loops).
    pub async fn list_wallets(&self) -> Result<Vec<Wallet>, StoreError> {
        let rows = sqlx::query_as::<_, Wallet>("SELECT * FROM wallets ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
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

    /// Fetch an address by id.
    pub async fn get_address(&self, id: Uuid) -> Result<Option<Address>, StoreError> {
        let row = sqlx::query_as::<_, Address>("SELECT * FROM addresses WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
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
                 source_account, destination_account, stellar_tx_hash, operation_index,
                 horizon_op_id, ledger, memo_id, status)
            VALUES ($1, $2, 'deposit', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'confirmed')
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
        .bind(&d.horizon_op_id)
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

    /// Update a withdrawal's status (and optional tx hash) after submission.
    pub async fn update_withdrawal_status(
        &self,
        id: Uuid,
        status: &str,
        stellar_tx_hash: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE withdrawals SET status = $2, stellar_tx_hash = $3, updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(status)
        .bind(stellar_tx_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
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

    // --- webhooks ---------------------------------------------------------

    /// Register a webhook endpoint for a wallet.
    pub async fn create_webhook_endpoint(
        &self,
        wallet_id: Uuid,
        url: &str,
        secret: &str,
    ) -> Result<WebhookEndpoint, StoreError> {
        sqlx::query_as::<_, WebhookEndpoint>(
            r#"
            INSERT INTO webhook_endpoints (wallet_id, url, secret)
            VALUES ($1, $2, $3)
            RETURNING *
            "#,
        )
        .bind(wallet_id)
        .bind(url)
        .bind(secret)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from_sqlx_conflict)
    }

    /// List the active webhook endpoints for a wallet.
    pub async fn active_webhook_endpoints(
        &self,
        wallet_id: Uuid,
    ) -> Result<Vec<WebhookEndpoint>, StoreError> {
        let rows = sqlx::query_as::<_, WebhookEndpoint>(
            "SELECT * FROM webhook_endpoints WHERE wallet_id = $1 AND active = true",
        )
        .bind(wallet_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // --- gas sponsorship --------------------------------------------------

    /// Read the gas sponsorship config for a wallet, if one exists.
    pub async fn get_gas_sponsorship_config(
        &self,
        wallet_id: Uuid,
    ) -> Result<Option<GasSponsorshipConfig>, StoreError> {
        let row = sqlx::query_as::<_, GasSponsorshipConfig>(
            "SELECT * FROM gas_sponsorship_configs WHERE wallet_id = $1",
        )
        .bind(wallet_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Insert a new sponsorship attempt with status `pending`. Returns the new row.
    ///
    /// Returns [`StoreError::Conflict`] if `inner_tx_hash` has already been sponsored (the unique
    /// constraint on `inner_tx_hash` fires), preventing double-sponsoring.
    pub async fn record_sponsored_tx(
        &self,
        new: NewSponsoredTx<'_>,
    ) -> Result<SponsoredTransaction, StoreError> {
        sqlx::query_as::<_, SponsoredTransaction>(
            r#"
            INSERT INTO sponsored_transactions (wallet_id, inner_tx_hash, fee_stroops)
            VALUES ($1, $2, $3)
            RETURNING *
            "#,
        )
        .bind(new.wallet_id)
        .bind(new.inner_tx_hash)
        .bind(new.fee_stroops)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from_sqlx_conflict)
    }

    /// Update a sponsored transaction's status and optional outcome fields after Horizon submission.
    pub async fn update_sponsored_tx_status(
        &self,
        id: Uuid,
        status: &str,
        fee_bump_tx_hash: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE sponsored_transactions
            SET status = $2, fee_bump_tx_hash = $3, error = $4
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(fee_bump_tx_hash)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Sum the `fee_stroops` of all **confirmed** sponsorships for `wallet_id` in the current UTC
    /// day. Returns `0` when no confirmed rows exist. Used for daily-budget enforcement.
    pub async fn sum_sponsored_fees_today(&self, wallet_id: Uuid) -> Result<i64, StoreError> {
        let total: i64 = sqlx::query_scalar(
            r#"
            SELECT COALESCE(SUM(fee_stroops), 0)::bigint
            FROM sponsored_transactions
            WHERE wallet_id = $1
              AND status = 'confirmed'
              AND date_trunc('day', created_at AT TIME ZONE 'UTC') =
                  date_trunc('day', now() AT TIME ZONE 'UTC')
            "#,
        )
        .bind(wallet_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(total)
    }

    /// Sum reserved fees (non-failed rows) for the current UTC day — used when displaying spend.
    pub async fn sum_sponsored_fees_reserved_today(
        &self,
        wallet_id: Uuid,
    ) -> Result<i64, StoreError> {
        let total: i64 = sqlx::query_scalar(
            r#"
            SELECT COALESCE(SUM(fee_stroops), 0)::bigint
            FROM sponsored_transactions
            WHERE wallet_id = $1
              AND status <> 'failed'
              AND date_trunc('day', created_at AT TIME ZONE 'UTC') =
                  date_trunc('day', now() AT TIME ZONE 'UTC')
            "#,
        )
        .bind(wallet_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(total)
    }

    /// Create or update gas sponsorship config for a wallet.
    pub async fn upsert_gas_sponsorship_config(
        &self,
        wallet_id: Uuid,
        enabled: bool,
        per_tx_fee_cap_stroops: Option<i64>,
        daily_budget_stroops: Option<i64>,
    ) -> Result<GasSponsorshipConfig, StoreError> {
        sqlx::query_as::<_, GasSponsorshipConfig>(
            r#"
            INSERT INTO gas_sponsorship_configs
                (wallet_id, enabled, per_tx_fee_cap_stroops, daily_budget_stroops)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (wallet_id) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                per_tx_fee_cap_stroops = EXCLUDED.per_tx_fee_cap_stroops,
                daily_budget_stroops = EXCLUDED.daily_budget_stroops,
                updated_at = now()
            RETURNING *
            "#,
        )
        .bind(wallet_id)
        .bind(enabled)
        .bind(per_tx_fee_cap_stroops)
        .bind(daily_budget_stroops)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::from)
    }

    /// Return true when a sponsored-transaction row already exists for `inner_tx_hash`.
    pub async fn sponsored_tx_exists(&self, inner_tx_hash: &str) -> Result<bool, StoreError> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sponsored_transactions WHERE inner_tx_hash = $1)",
        )
        .bind(inner_tx_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    /// Atomically check the daily sponsorship budget and reserve it in one statement.
    ///
    /// Returns `Ok(None)` when the budget would be exceeded or `inner_tx_hash` was already recorded
    /// (`ON CONFLICT DO NOTHING`). Callers distinguish duplicate vs budget via
    /// [`Self::sponsored_tx_exists`].
    pub async fn record_sponsored_tx_if_budget_available(
        &self,
        wallet_id: Uuid,
        inner_tx_hash: &str,
        fee_stroops: i64,
        daily_budget_stroops: i64,
    ) -> Result<Option<SponsoredTransaction>, StoreError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(wallet_id)
            .execute(&mut *tx)
            .await?;

        let row = sqlx::query_as::<_, SponsoredTransaction>(
            r#"
            WITH budget_check AS (
                SELECT COALESCE(SUM(fee_stroops), 0)::bigint AS spent_today
                FROM sponsored_transactions
                WHERE wallet_id = $1
                  AND status <> 'failed'
                  AND date_trunc('day', created_at AT TIME ZONE 'UTC') =
                      date_trunc('day', now() AT TIME ZONE 'UTC')
            )
            INSERT INTO sponsored_transactions (wallet_id, inner_tx_hash, fee_stroops, status)
            SELECT $1, $2, $3, 'pending'
            FROM budget_check
            WHERE spent_today + $3 <= $4
            ON CONFLICT (inner_tx_hash) DO NOTHING
            RETURNING *
            "#,
        )
        .bind(wallet_id)
        .bind(inner_tx_hash)
        .bind(fee_stroops)
        .bind(daily_budget_stroops)
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(row)
    }

    /// Record a webhook delivery attempt (audit log). Returns the delivery id.
    pub async fn log_webhook_delivery(
        &self,
        endpoint_id: Uuid,
        event_type: &str,
        payload: &serde_json::Value,
        status: &str,
        attempts: i32,
        response_code: Option<i32>,
    ) -> Result<Uuid, StoreError> {
        let id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO webhook_deliveries
                (endpoint_id, event_type, payload, status, attempts, response_code)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(endpoint_id)
        .bind(event_type)
        .bind(payload)
        .bind(status)
        .bind(attempts)
        .bind(response_code)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }
}

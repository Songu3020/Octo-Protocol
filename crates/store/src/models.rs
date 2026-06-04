//! Typed row models mirroring the schema in `migrations/0001_init.sql`.
//!
//! Amounts are `i64` stroops throughout (never floating point). Sealed-seed bytes are stored as
//! `Vec<u8>` and only ever decrypted inside `octo-wallet-core`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A master wallet (one per network), holding the sealed HD seed.
#[derive(Debug, Clone, FromRow)]
pub struct Wallet {
    pub id: Uuid,
    pub network: String,
    pub stellar_account_g: String,
    pub sealed_ciphertext: Vec<u8>,
    pub sealed_nonce: Vec<u8>,
    pub sealed_salt: Vec<u8>,
    pub next_muxed_id: i64,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A per-customer deposit address (off-chain row).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Address {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub muxed_id: i64,
    pub muxed_address: String,
    pub customer_ref: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A deposit or withdrawal ledger entry.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub address_id: Option<Uuid>,
    pub direction: String,
    pub asset_code: String,
    pub asset_issuer: Option<String>,
    pub amount_stroops: i64,
    pub source_account: Option<String>,
    pub destination_account: Option<String>,
    pub stellar_tx_hash: Option<String>,
    pub operation_index: Option<i32>,
    pub ledger: Option<i64>,
    pub memo_id: Option<i64>,
    pub status: String,
    pub reference: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A withdrawal (payout) intent.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Withdrawal {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub idempotency_key: String,
    pub destination_account: String,
    pub asset_code: String,
    pub asset_issuer: Option<String>,
    pub amount_stroops: i64,
    pub memo_id: Option<i64>,
    pub status: String,
    pub stellar_tx_hash: Option<String>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A registered webhook endpoint.
#[derive(Debug, Clone, FromRow)]
pub struct WebhookEndpoint {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub url: String,
    pub secret: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// A new deposit to record (input to the idempotent insert).
#[derive(Debug, Clone)]
pub struct NewDeposit {
    pub wallet_id: Uuid,
    pub address_id: Option<Uuid>,
    pub asset_code: String,
    pub asset_issuer: Option<String>,
    pub amount_stroops: i64,
    pub source_account: Option<String>,
    pub destination_account: Option<String>,
    pub stellar_tx_hash: String,
    pub operation_index: i32,
    pub ledger: Option<i64>,
    pub memo_id: Option<i64>,
}

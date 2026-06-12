//! Horizon payments client for the ingest worker.
//!
//! Polls an account's `/payments` endpoint (with `join=transactions` so we get the memo) using a
//! saved paging-token cursor. Cursor polling — rather than the SSE stream — keeps the worker
//! simple, restart-safe, and trivially horizontally scalable (one worker per account); the cursor
//! is the durable resume point.

use serde::Deserialize;

/// Errors talking to Horizon.
#[derive(Debug, thiserror::Error)]
pub enum HorizonError {
    #[error("horizon request failed")]
    Request,
    #[error("horizon returned an unexpected response")]
    Decode,
}

/// One payment record from Horizon (the fields octo needs).
#[derive(Debug, Clone, Deserialize)]
pub struct PaymentRecord {
    /// The operation TOID — globally unique; used as the idempotent dedup key.
    pub id: String,
    /// Cursor token for resuming after this record.
    pub paging_token: String,
    /// `"payment"` or `"create_account"` etc. We only credit `payment` (and createAccount).
    #[serde(rename = "type")]
    pub kind: String,
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub transaction_successful: bool,
    pub from: Option<String>,
    /// Destination base account (`G...`).
    pub to: Option<String>,
    /// Present when the payment was sent to a muxed (`M...`) address.
    #[serde(default)]
    pub to_muxed: Option<String>,
    /// The muxed id (customer id) when `to_muxed` is set. Horizon returns it as a string.
    #[serde(default)]
    pub to_muxed_id: Option<String>,
    pub asset_type: Option<String>,
    #[serde(default)]
    pub asset_code: Option<String>,
    #[serde(default)]
    pub asset_issuer: Option<String>,
    /// Decimal amount string, e.g. "10.0000000".
    pub amount: Option<String>,
    /// createAccount uses `starting_balance` instead of `amount`.
    #[serde(default)]
    pub starting_balance: Option<String>,
    /// Joined parent transaction (for memo + ledger).
    #[serde(default)]
    pub transaction: Option<TransactionRecord>,
}

/// The joined transaction fields we use.
#[derive(Debug, Clone, Deserialize)]
pub struct TransactionRecord {
    #[serde(default)]
    pub memo_type: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub ledger: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Embedded {
    records: Vec<PaymentRecord>,
}

#[derive(Debug, Deserialize)]
struct PaymentsPage {
    _embedded: Embedded,
}

/// A thin Horizon payments client.
#[derive(Clone)]
pub struct HorizonPayments {
    http: reqwest::Client,
    base_url: String,
}

impl HorizonPayments {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Fetch up to `limit` payments for `account_g`, oldest-first, starting after `cursor`.
    ///
    /// Oldest-first (`order=asc`) so we process and advance the cursor monotonically.
    pub async fn payments_after(
        &self,
        account_g: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<Vec<PaymentRecord>, HorizonError> {
        let mut url = format!(
            "{}/accounts/{}/payments?order=asc&limit={}&join=transactions",
            self.base_url.trim_end_matches('/'),
            account_g,
            limit
        );
        if let Some(c) = cursor {
            url.push_str("&cursor=");
            url.push_str(c);
        }

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|_| HorizonError::Request)?;

        // A 404 means the account does not exist on-chain yet (e.g. a wallet created but not yet
        // funded). That is not an error for ingestion — there simply are no payments to process.
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !resp.status().is_success() {
            return Err(HorizonError::Request);
        }
        let page: PaymentsPage = resp.json().await.map_err(|_| HorizonError::Decode)?;
        Ok(page._embedded.records)
    }
}

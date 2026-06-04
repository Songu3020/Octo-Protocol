//! Minimal Horizon + friendbot client used by the API for funding and balance reads.
//!
//! Only the few endpoints octo needs are implemented. Network errors map to `ApiError::Internal`
//! (logged by the caller); a missing account maps to `ApiError::NotFound`.

use crate::error::ApiError;
use serde::{Deserialize, Serialize};

/// A single balance line from a Horizon account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    /// Decimal string, e.g. "100.0000000".
    pub balance: String,
    /// "native" for XLM, else "credit_alphanum4" / "credit_alphanum12".
    pub asset_type: String,
    #[serde(default)]
    pub asset_code: Option<String>,
    #[serde(default)]
    pub asset_issuer: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountResponse {
    balances: Vec<Balance>,
}

/// A thin Horizon client (one shared reqwest client).
#[derive(Clone)]
pub struct Horizon {
    http: reqwest::Client,
    base_url: String,
}

impl Horizon {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Fetch an account's balances. Returns `NotFound` if the account does not exist on-chain yet.
    pub async fn balances(&self, account_g: &str) -> Result<Vec<Balance>, ApiError> {
        let url = format!(
            "{}/accounts/{}",
            self.base_url.trim_end_matches('/'),
            account_g
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|_| ApiError::Internal)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ApiError::NotFound);
        }
        if !resp.status().is_success() {
            return Err(ApiError::Internal);
        }
        let account: AccountResponse = resp.json().await.map_err(|_| ApiError::Internal)?;
        Ok(account.balances)
    }
}

/// Fund a testnet account via friendbot. Best-effort: returns `Ok(())` on success, and a logged
/// error otherwise (the caller decides whether funding is required).
pub async fn friendbot_fund(friendbot_url: &str, account_g: &str) -> Result<(), ApiError> {
    let url = format!(
        "{}/?addr={}",
        friendbot_url.trim_end_matches('/'),
        account_g
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|_| ApiError::Internal)?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(ApiError::Internal)
    }
}

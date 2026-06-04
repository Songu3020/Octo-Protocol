//! Wallet endpoints: create a master wallet, fetch one.

use crate::error::{ApiError, ApiResult, Envelope};
use crate::json::parse_optional;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use octo_store::NewWallet;
use octo_wallet_core::provision_wallet;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Optional body for wallet creation.
#[derive(Debug, Default, Deserialize)]
pub struct CreateWalletRequest {
    /// Optional human label for the wallet.
    #[serde(default)]
    pub label: Option<String>,
}

/// What we return after creating a wallet. The mnemonic is returned **once** here so the operator
/// can back it up; it is never stored in plaintext and never returned again.
#[derive(Debug, Serialize)]
pub struct CreateWalletResponse {
    pub id: Uuid,
    pub network: String,
    pub address: String,
    /// One-time recovery mnemonic — store this securely; it will not be shown again.
    pub recovery_mnemonic: String,
}

/// Public wallet view (no secrets).
#[derive(Debug, Serialize)]
pub struct WalletView {
    pub id: Uuid,
    pub network: String,
    pub address: String,
    pub label: Option<String>,
}

/// `POST /v1/wallets`
pub async fn create_wallet(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Envelope<CreateWalletResponse>>)> {
    let req: CreateWalletRequest = parse_optional(&body)?;
    let label = req.label;

    // Generate + seal in wallet-core; the raw seed never reaches this layer.
    let provisioned = provision_wallet(state.master_key(), state.network())?;

    let wallet = state
        .store()
        .create_wallet(NewWallet {
            network: state.network().as_str(),
            stellar_account_g: &provisioned.account_g,
            sealed_ciphertext: &provisioned.sealed.ciphertext,
            sealed_nonce: &provisioned.sealed.nonce,
            sealed_salt: &provisioned.sealed.salt,
            label: label.as_deref(),
        })
        .await?;

    let resp = CreateWalletResponse {
        id: wallet.id,
        network: wallet.network,
        address: wallet.stellar_account_g,
        recovery_mnemonic: provisioned.mnemonic.to_string(),
    };
    let (status, json) = Envelope::created(resp);
    Ok((status, json))
}

/// `GET /v1/wallets/{id}`
pub async fn get_wallet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Envelope<WalletView>>> {
    let w = state.store().get_wallet(id).await.map_err(|e| match e {
        octo_store::StoreError::NotFound => ApiError::NotFound,
        _ => ApiError::Internal,
    })?;
    Ok(Envelope::ok(WalletView {
        id: w.id,
        network: w.network,
        address: w.stellar_account_g,
        label: w.label,
    }))
}

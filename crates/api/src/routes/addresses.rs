//! Address endpoints: generate a customer deposit address, list them.
//!
//! Each address is returned in **both** forms — the muxed `M...` (default) and the
//! `G...` + numeric `memo_id` fallback for senders that don't support muxed (see
//! `docs/deposit-model.md`).

use crate::error::{ApiResult, Envelope};
use crate::json::parse_optional;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use octo_wallet_core::encode_muxed;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for creating an address.
#[derive(Debug, Default, Deserialize)]
pub struct CreateAddressRequest {
    /// Opaque caller reference for their own user.
    #[serde(default)]
    pub customer_ref: Option<String>,
    /// Arbitrary metadata echoed back in webhooks.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// An address in both deposit forms.
#[derive(Debug, Serialize)]
pub struct AddressView {
    pub id: Uuid,
    pub customer_ref: Option<String>,
    /// Default form handed to customers.
    pub muxed_address: String,
    /// Fallback for `G...`+memo senders (same id as the muxed address).
    pub base_address: String,
    pub memo_id: i64,
    pub metadata: serde_json::Value,
}

/// `POST /v1/wallets/{id}/addresses`
pub async fn create_address(
    State(state): State<AppState>,
    Path(wallet_id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Envelope<AddressView>>)> {
    let req: CreateAddressRequest = parse_optional(&body)?;

    // Fetch the wallet to learn its base G... account (the muxed addresses encode it).
    let wallet = state.store().get_wallet(wallet_id).await?;
    let base = wallet.stellar_account_g.clone();

    let metadata = req.metadata.unwrap_or_else(|| serde_json::json!({}));

    // allocate_address bumps the muxed-id counter atomically and derives the M... via this closure.
    let address = state
        .store()
        .allocate_address(
            wallet_id,
            |id| {
                // muxed_id is a positive i64 from the counter; encode needs u64.
                let id_u64 = u64::try_from(id).map_err(|_| ())?;
                encode_muxed(&base, id_u64).map_err(|_| ())
            },
            req.customer_ref.as_deref(),
            metadata,
        )
        .await?;

    let view = AddressView {
        id: address.id,
        customer_ref: address.customer_ref,
        muxed_address: address.muxed_address,
        base_address: wallet.stellar_account_g,
        memo_id: address.muxed_id,
        metadata: address.metadata,
    };
    let (status, json) = Envelope::created(view);
    Ok((status, json))
}

/// `GET /v1/wallets/{id}/addresses`
pub async fn list_addresses(
    State(state): State<AppState>,
    Path(wallet_id): Path<Uuid>,
) -> ApiResult<Json<Envelope<Vec<AddressView>>>> {
    // Confirm the wallet exists (404 otherwise) and get its base account for the fallback form.
    let wallet = state.store().get_wallet(wallet_id).await?;
    let rows = state.store().list_addresses(wallet_id).await?;

    let views = rows
        .into_iter()
        .map(|a| AddressView {
            id: a.id,
            customer_ref: a.customer_ref,
            muxed_address: a.muxed_address,
            base_address: wallet.stellar_account_g.clone(),
            memo_id: a.muxed_id,
            metadata: a.metadata,
        })
        .collect();

    Ok(Envelope::ok(views))
}

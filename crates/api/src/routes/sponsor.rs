//! Fee-bump sponsorship endpoint.
//!
//! Receives a signed user transaction (inner XDR), validates it against the wallet's
//! sponsorship policy, wraps it in a fee-bump signed by the master key, and submits
//! the outer envelope to Horizon.

use crate::error::{ApiError, ApiResult, Envelope};
use crate::json::parse_optional;
use crate::sponsor_validation::validate_inner_xdr;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use octo_crypto::SealedSeed;
use octo_wallet_core::{compute_inner_tx_hash, sign_fee_bump, FeeBumpRequest};
use octo_webhooks::Event;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Webhook event type fired after every sponsor request.
pub const SPONSORED_EVENT_TYPE: &str = "transaction.sponsored";

#[derive(Debug, Default, Deserialize)]
pub struct SponsorRequest {
    /// Base64-encoded `TransactionEnvelope` XDR of the user's signed inner transaction.
    pub transaction_xdr: Option<String>,
    /// Total fee (in stroops) the master wallet is willing to pay for the fee-bump.
    pub max_base_fee_stroops: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SponsorResponse {
    pub id: Uuid,
    pub status: String,
    pub inner_tx_hash: String,
    pub fee_bump_tx_hash: Option<String>,
    pub fee_stroops: i64,
}

/// `POST /v1/wallets/:id/sponsor`
pub async fn sponsor(
    State(state): State<AppState>,
    Path(wallet_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Envelope<SponsorResponse>>)> {
    crate::auth::authorize_wallet(&headers, &state, wallet_id).await?;

    let req: SponsorRequest = parse_optional(&body)?;

    let inner_xdr = req
        .transaction_xdr
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("transaction_xdr is required".into()))?;

    let max_fee = req
        .max_base_fee_stroops
        .filter(|f| *f > 0)
        .ok_or_else(|| ApiError::BadRequest("max_base_fee_stroops must be > 0".into()))?;

    let wallet = state.store().get_wallet(wallet_id).await?;

    let config = state
        .store()
        .get_gas_sponsorship_config(wallet_id)
        .await
        .map_err(|_| ApiError::Internal)?
        .ok_or_else(|| {
            ApiError::Forbidden("gas sponsorship is not configured for this wallet".into())
        })?;

    if !config.enabled {
        return Err(ApiError::Forbidden(
            "gas sponsorship is disabled for this wallet".into(),
        ));
    }

    if let Some(cap) = config.per_tx_fee_cap_stroops {
        if max_fee > cap {
            return Err(ApiError::Forbidden(format!(
                "requested fee ({max_fee} stroops) exceeds the per-transaction cap ({cap} stroops)"
            )));
        }
    }

    validate_inner_xdr(&inner_xdr, &wallet.stellar_account_g)?;

    let hash_bytes = compute_inner_tx_hash(&inner_xdr, state.network())
        .map_err(|_| ApiError::BadRequest("invalid transaction XDR".into()))?;
    let inner_tx_hash = hex::encode(hash_bytes);

    let record = if let Some(budget) = config.daily_budget_stroops {
        match state
            .store()
            .record_sponsored_tx_if_budget_available(wallet_id, &inner_tx_hash, max_fee, budget)
            .await?
        {
            Some(row) => row,
            None => {
                if state.store().sponsored_tx_exists(&inner_tx_hash).await? {
                    return Err(ApiError::Conflict);
                }
                return Err(ApiError::TooManyRequests(format!(
                    "daily sponsorship budget of {budget} stroops would be exceeded"
                )));
            }
        }
    } else {
        state
            .store()
            .record_sponsored_tx(octo_store::NewSponsoredTx {
                wallet_id,
                inner_tx_hash: &inner_tx_hash,
                fee_stroops: max_fee,
            })
            .await?
    };

    let sealed = SealedSeed::from_parts(
        wallet.sealed_ciphertext.clone(),
        &wallet.sealed_nonce,
        &wallet.sealed_salt,
    )
    .map_err(|_| ApiError::Internal)?;

    let signed = sign_fee_bump(
        state.master_key(),
        &sealed,
        state.network(),
        0,
        &FeeBumpRequest {
            inner_xdr: &inner_xdr,
            max_base_fee_stroops: max_fee,
        },
    )?;

    let submit = state
        .horizon()
        .submit_transaction(&signed.envelope_xdr)
        .await;

    let (status, fee_bump_hash, err_msg) = match submit {
        Ok(r) if r.successful => ("confirmed", Some(r.hash), None),
        Ok(r) => (
            "failed",
            Some(r.hash),
            Some("transaction failed on-chain".to_string()),
        ),
        Err(_) => ("failed", None, Some("Horizon submission error".to_string())),
    };

    let _ = state
        .store()
        .update_sponsored_tx_status(
            record.id,
            status,
            fee_bump_hash.as_deref(),
            err_msg.as_deref(),
        )
        .await;

    if let Some(user_id) = wallet.user_id {
        crate::audit::record(
            &state,
            user_id,
            &format!("sponsored a fee-bump transaction ({status})"),
            crate::audit::category::SPONSORSHIP,
            Some(&inner_tx_hash),
            &headers,
        )
        .await;
    }

    fire_sponsored_event(
        state.clone(),
        wallet_id,
        inner_tx_hash.clone(),
        fee_bump_hash.clone(),
        max_fee,
        status,
    );

    let resp = SponsorResponse {
        id: record.id,
        status: status.to_string(),
        inner_tx_hash,
        fee_bump_tx_hash: fee_bump_hash,
        fee_stroops: max_fee,
    };
    let (code, json) = Envelope::created(resp);
    Ok((code, json))
}

fn fire_sponsored_event(
    state: AppState,
    wallet_id: Uuid,
    inner_tx_hash: String,
    fee_bump_tx_hash: Option<String>,
    fee_stroops: i64,
    status: &'static str,
) {
    tokio::spawn(async move {
        let event = Event {
            event_type: SPONSORED_EVENT_TYPE.to_string(),
            data: serde_json::json!({
                "wallet_id": wallet_id,
                "inner_tx_hash": inner_tx_hash,
                "fee_bump_tx_hash": fee_bump_tx_hash,
                "fee_stroops": fee_stroops,
                "status": status,
                "created_at": chrono::Utc::now(),
            }),
        };
        state.webhooks().dispatch(wallet_id, &event).await;
    });
}

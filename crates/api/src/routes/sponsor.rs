//! `POST /v1/wallets/:id/sponsor` — submit a user-signed transaction for the wallet's master
//! account to pay the network fee (a Stellar fee-bump), subject to the wallet's sponsorship
//! policy: enabled flag, per-operation fee cap, daily budget, and operation allowlist.
//!
//! Every outcome — confirmed, failed, or policy rejection — is recorded in the audit log (see
//! `crate::audit`) so sponsorship usage and abuse attempts are always traceable. Rejections are
//! logged **before** the error response is returned.

use crate::auth::authorize_wallet;
use crate::error::{ApiError, ApiResult, Envelope};
use crate::json::parse_optional;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use octo_crypto::SealedSeed;
use octo_wallet_core::{FeeBumpRequest, WalletError, MIN_FEE_PER_OP_STROOPS};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Deserialize)]
pub struct SponsorRequest {
    pub transaction_xdr: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SponsorResponse {
    pub status: String,
    pub inner_tx_hash: String,
    pub fee_stroops: i64,
    pub stellar_tx_hash: Option<String>,
}

/// Record the abuse-trail audit entry for a policy rejection, then return the error response.
/// Best-effort: recording never blocks the rejection from being returned.
async fn reject(
    state: &AppState,
    user_id: Option<Uuid>,
    wallet_id: Uuid,
    headers: &HeaderMap,
    reason: &str,
) -> ApiError {
    if let Some(uid) = user_id {
        crate::audit::record(
            state,
            uid,
            &format!("sponsor request rejected: {reason}"),
            crate::audit::category::SPONSORSHIP,
            Some(&wallet_id.to_string()),
            headers,
        )
        .await;
    }
    ApiError::BadRequest(format!("sponsor request rejected: {reason}"))
}

/// `POST /v1/wallets/:id/sponsor`
pub async fn sponsor(
    State(state): State<AppState>,
    Path(wallet_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Envelope<SponsorResponse>>)> {
    authorize_wallet(&headers, &state, wallet_id).await?;
    let req: SponsorRequest = parse_optional(&body)?;
    let wallet = state.store().get_wallet(wallet_id).await?;
    let user_id = wallet.user_id;

    let transaction_xdr = req
        .transaction_xdr
        .filter(|x| !x.is_empty())
        .ok_or_else(|| ApiError::BadRequest("transaction_xdr is required".into()))?;

    // --- policy: sponsorship must be enabled for this wallet ---
    let config = state.store().get_sponsorship_config(wallet_id).await?;
    let Some(config) = config.filter(|c| c.enabled) else {
        return Err(reject(&state, user_id, wallet_id, &headers, "sponsorship_disabled").await);
    };

    let fee_per_op = if config.fee_cap_stroops > 0 {
        config.fee_cap_stroops
    } else {
        MIN_FEE_PER_OP_STROOPS
    };

    // --- parse + sign the fee-bump (this also enforces the op-type allowlist) ---
    let sealed = SealedSeed::from_parts(
        wallet.sealed_ciphertext.clone(),
        &wallet.sealed_nonce,
        &wallet.sealed_salt,
    )
    .map_err(|_| ApiError::Internal)?;

    let fee_bump_req = FeeBumpRequest {
        inner_tx_xdr: &transaction_xdr,
        fee_per_op_stroops: fee_per_op,
    };
    let signed = match octo_wallet_core::sign_fee_bump(
        state.master_key(),
        &sealed,
        state.network(),
        0,
        &fee_bump_req,
    ) {
        Ok(s) => s,
        Err(WalletError::InvalidXdr) => {
            return Err(reject(&state, user_id, wallet_id, &headers, "xdr_invalid").await)
        }
        Err(WalletError::OperationNotAllowed) => {
            return Err(reject(&state, user_id, wallet_id, &headers, "op_not_allowed").await)
        }
        Err(_) => return Err(ApiError::Internal),
    };

    // --- policy: stay within the wallet's daily sponsorship budget (atomic reservation) ---
    let reserved = state
        .store()
        .try_reserve_sponsorship_budget(wallet_id, signed.fee_stroops)
        .await?;
    if reserved.is_none() {
        return Err(reject(&state, user_id, wallet_id, &headers, "budget_exceeded").await);
    }

    // --- submit to Horizon ---
    let submit = state
        .horizon()
        .submit_transaction(&signed.envelope_xdr)
        .await;
    let (status, stellar_tx_hash) = match submit {
        Ok(r) if r.successful => ("confirmed", Some(r.hash)),
        Ok(r) => ("failed", Some(r.hash)),
        Err(_) => ("failed", None),
    };

    // Idempotent on (wallet_id, inner_tx_hash) — a resubmitted XDR is recorded once.
    let _ = state
        .store()
        .create_sponsored_transaction(
            wallet_id,
            &signed.inner_tx_hash,
            signed.fee_stroops,
            status,
            stellar_tx_hash.as_deref(),
        )
        .await;

    if let Some(uid) = user_id {
        crate::audit::record(
            &state,
            uid,
            &format!("sponsored a transaction ({status})"),
            crate::audit::category::SPONSORSHIP,
            Some(&wallet_id.to_string()),
            &headers,
        )
        .await;
    }

    let resp = SponsorResponse {
        status: status.to_string(),
        inner_tx_hash: signed.inner_tx_hash,
        fee_stroops: signed.fee_stroops,
        stellar_tx_hash,
    };
    let (code, json) = Envelope::created(resp);
    Ok((code, json))
}

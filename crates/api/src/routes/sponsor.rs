//! Fee-bump sponsorship endpoint.
//!
//! Receives a signed user transaction (inner XDR), validates it against the wallet's
//! sponsorship policy, wraps it in a fee-bump signed by the master key, and submits
//! the outer envelope to Horizon.
//!
//! Security notes:
//! - Auth accepts both JWT and API key (`authorize_wallet`), matching integration use.
//! - The inner tx source must not be the master account (self-sponsorship guard).
//! - Only Payment / PathPaymentStrictSend / PathPaymentStrictReceive are allowed op types.
//! - The UNIQUE constraint on `inner_tx_hash` prevents double-sponsoring the same tx.
//! - Per-tx fee cap and daily budget are enforced before signing.

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
use octo_store::NewSponsoredTx;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Deserialize)]
pub struct SponsorRequest {
    /// Base64-encoded `TransactionEnvelope` XDR of the user's signed inner transaction.
    pub transaction_xdr: Option<String>,
    /// Total fee (in stroops) the master wallet is willing to pay for the fee-bump.
    /// Must be > 0 and within the wallet's `per_tx_fee_cap_stroops` (if configured).
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
    // Accepts both JWT (dashboard) and API key (integration). API keys may drive sponsorship.
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

    // Load wallet (needed for sealed seed + master G address for self-sponsorship check).
    let wallet = state.store().get_wallet(wallet_id).await?;

    // --- Sponsorship policy checks ---

    let config = state
        .store()
        .get_gas_sponsorship_config(wallet_id)
        .await
        .map_err(|_| ApiError::Internal)?;

    let config = config.ok_or_else(|| {
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

    if let Some(budget) = config.daily_budget_stroops {
        let spent_today = state
            .store()
            .sum_sponsored_fees_today(wallet_id)
            .await
            .map_err(|_| ApiError::Internal)?;
        if spent_today + max_fee > budget {
            return Err(ApiError::TooManyRequests(format!(
                "daily sponsorship budget of {budget} stroops would be exceeded \
                 ({spent_today} stroops spent today)"
            )));
        }
    }

    // --- XDR validation (op allowlist + self-sponsorship check) ---
    validate_inner_xdr(&inner_xdr, &wallet.stellar_account_g)?;

    // --- Compute the inner tx hash (deterministic; used as the dedup key) ---
    let hash_bytes =
        compute_inner_tx_hash(&inner_xdr, state.network()).map_err(|_| ApiError::BadRequest("invalid transaction XDR".into()))?;
    let inner_tx_hash = hex::encode(hash_bytes);

    // --- Record pending row (UNIQUE on inner_tx_hash prevents double-sponsoring) ---
    let record = state
        .store()
        .record_sponsored_tx(NewSponsoredTx {
            wallet_id,
            inner_tx_hash: &inner_tx_hash,
            fee_stroops: max_fee,
        })
        .await?; // StoreError::Conflict -> ApiError::Conflict (409)

    // --- Sign the fee-bump inside wallet-core (decrypt -> derive -> sign -> zeroize) ---
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

    // --- Submit to Horizon ---
    let submit = state
        .horizon()
        .submit_transaction(&signed.envelope_xdr)
        .await;

    let (status, fee_bump_hash, err_msg) = match submit {
        Ok(r) if r.successful => ("confirmed", Some(r.hash), None),
        Ok(r) => ("failed", Some(r.hash), Some("transaction failed on-chain".to_string())),
        Err(_) => ("failed", None, Some("Horizon submission error".to_string())),
    };

    // --- Update the record (best-effort; never blocks the response) ---
    let _ = state
        .store()
        .update_sponsored_tx_status(record.id, status, fee_bump_hash.as_deref(), err_msg.as_deref())
        .await;

    // --- Audit log (best-effort; only when a user identity is available) ---
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

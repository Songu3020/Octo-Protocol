//! Gas sponsorship: atomic daily spend-limit enforcement.
//!
//! This is intentionally a thin slice of the full sponsorship feature (no XDR validation, fee-bump
//! signing, or Horizon submission — those are tracked separately). It exists to host the one thing
//! this issue is about: an atomic, race-free daily budget check.
//!
//! Security (see `docs/threat-model.md`):
//! - [`octo_store::Store::record_sponsored_tx_if_budget_available`] checks and reserves the budget
//!   in a single conditional-insert statement, so concurrent requests can't both read the same
//!   "budget available" snapshot and jointly overrun it.
//! - Idempotent on `inner_tx_hash`: a retried request for an already-recorded inner transaction is
//!   a no-op, never a second reservation.

use crate::auth::authorize_wallet;
use crate::error::{ApiError, ApiResult, Envelope};
use crate::json::parse_optional;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use octo_store::StoreError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for sponsoring a transaction's fee.
#[derive(Debug, Default, Deserialize)]
pub struct SponsorRequest {
    /// Hash of the user's inner transaction (idempotency key — prevents double-sponsoring).
    pub inner_tx_hash: Option<String>,
    /// Fee to reserve against the daily budget, in stroops.
    pub fee_stroops: Option<i64>,
    /// Daily sponsorship budget for this wallet, in stroops. Caller-supplied for now: there is no
    /// persisted per-wallet sponsorship config in scope yet.
    pub daily_budget_stroops: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SponsoredTxView {
    pub id: Uuid,
    pub inner_tx_hash: String,
    pub fee_stroops: i64,
    pub status: String,
}

/// `POST /v1/wallets/{id}/sponsor`
pub async fn sponsor(
    State(state): State<AppState>,
    Path(wallet_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Envelope<SponsoredTxView>>)> {
    authorize_wallet(&headers, &state, wallet_id).await?;
    let req: SponsorRequest = parse_optional(&body)?;

    let inner_tx_hash = req
        .inner_tx_hash
        .filter(|h| !h.is_empty())
        .ok_or_else(|| ApiError::BadRequest("inner_tx_hash is required".into()))?;
    let fee_stroops = req
        .fee_stroops
        .filter(|f| *f > 0)
        .ok_or_else(|| ApiError::BadRequest("fee_stroops must be > 0".into()))?;
    let daily_budget_stroops = req
        .daily_budget_stroops
        .filter(|b| *b > 0)
        .ok_or_else(|| ApiError::BadRequest("daily_budget_stroops must be > 0".into()))?;

    let wallet = state.store().get_wallet(wallet_id).await?;

    let reserved = state
        .store()
        .record_sponsored_tx_if_budget_available(
            wallet_id,
            &inner_tx_hash,
            fee_stroops,
            daily_budget_stroops,
        )
        .await?
        .ok_or(StoreError::BudgetExceeded)?;

    if let Some(uid) = wallet.user_id {
        crate::audit::record(
            &state,
            uid,
            "reserved gas sponsorship budget",
            crate::audit::category::SPONSORSHIP,
            Some(&inner_tx_hash),
            &headers,
        )
        .await;
    }

    let view = SponsoredTxView {
        id: reserved.id,
        inner_tx_hash: reserved.inner_tx_hash,
        fee_stroops: reserved.fee_stroops,
        status: reserved.status,
    };
    let (status, json) = Envelope::created(view);
    Ok((status, json))
}

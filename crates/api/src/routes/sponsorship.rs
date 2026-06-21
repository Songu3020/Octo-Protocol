//! Gas sponsorship configuration: enable/disable sponsorship for a wallet and set its spend
//! controls (per-operation fee cap, daily budget).

use crate::auth::authorize_wallet;
use crate::error::{ApiError, ApiResult, Envelope};
use crate::json::parse_optional;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Deserialize)]
pub struct SponsorshipConfigRequest {
    pub enabled: Option<bool>,
    pub fee_cap_stroops: Option<i64>,
    pub daily_budget_stroops: Option<i64>,
}

#[derive(Debug, Default, Serialize)]
pub struct SponsorshipConfigView {
    pub enabled: bool,
    pub fee_cap_stroops: i64,
    pub daily_budget_stroops: i64,
    pub spent_today_stroops: i64,
}

impl From<octo_store::GasSponsorshipConfig> for SponsorshipConfigView {
    fn from(c: octo_store::GasSponsorshipConfig) -> Self {
        Self {
            enabled: c.enabled,
            fee_cap_stroops: c.fee_cap_stroops,
            daily_budget_stroops: c.daily_budget_stroops,
            spent_today_stroops: c.spent_today_stroops,
        }
    }
}

/// `GET /v1/wallets/:id/sponsorship`
pub async fn get_config(
    State(state): State<AppState>,
    Path(wallet_id): Path<Uuid>,
    headers: HeaderMap,
) -> ApiResult<Json<Envelope<SponsorshipConfigView>>> {
    authorize_wallet(&headers, &state, wallet_id).await?;
    let view = state
        .store()
        .get_sponsorship_config(wallet_id)
        .await?
        .map(SponsorshipConfigView::from)
        .unwrap_or_default();
    Ok(Envelope::ok(view))
}

/// `PUT /v1/wallets/:id/sponsorship`
pub async fn put_config(
    State(state): State<AppState>,
    Path(wallet_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Envelope<SponsorshipConfigView>>> {
    authorize_wallet(&headers, &state, wallet_id).await?;
    let req: SponsorshipConfigRequest = parse_optional(&body)?;

    let enabled = req.enabled.unwrap_or(false);
    let fee_cap_stroops = req
        .fee_cap_stroops
        .filter(|v| *v >= 0)
        .ok_or_else(|| ApiError::BadRequest("fee_cap_stroops must be >= 0".into()))?;
    let daily_budget_stroops = req
        .daily_budget_stroops
        .filter(|v| *v >= 0)
        .ok_or_else(|| ApiError::BadRequest("daily_budget_stroops must be >= 0".into()))?;

    let config = state
        .store()
        .upsert_sponsorship_config(wallet_id, enabled, fee_cap_stroops, daily_budget_stroops)
        .await?;

    // Confirm the wallet exists and learn its owner for the audit entry.
    let wallet = state.store().get_wallet(wallet_id).await?;
    if let Some(uid) = wallet.user_id {
        crate::audit::record(
            &state,
            uid,
            &format!("updated sponsorship config (enabled: {enabled})"),
            crate::audit::category::SPONSORSHIP,
            Some(&wallet_id.to_string()),
            &headers,
        )
        .await;
    }

    Ok(Envelope::ok(config.into()))
}

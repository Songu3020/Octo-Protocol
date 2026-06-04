//! axum REST API for octo: wallets, addresses (and withdrawals/webhooks in later steps).
//!
//! All responses use the `{ statusCode, message, data }` envelope (see [`error`]). State and
//! secret handling live in [`state`]; routes never touch raw seed material.
#![forbid(unsafe_code)]

mod error;
mod json;
pub mod routes;
mod state;

pub use error::{ApiError, ApiResult, Envelope};
pub use state::AppState;

use axum::routing::{get, post};
use axum::Router;

/// Build the API router with shared state.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/wallets", post(routes::wallets::create_wallet))
        .route("/v1/wallets/:id", get(routes::wallets::get_wallet))
        .route(
            "/v1/wallets/:id/addresses",
            post(routes::addresses::create_address).get(routes::addresses::list_addresses),
        )
        .with_state(state)
}

/// Liveness probe.
async fn health() -> &'static str {
    "ok"
}

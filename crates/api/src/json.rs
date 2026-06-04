//! Lenient JSON body parsing.
//!
//! Endpoints accept an optional JSON body: an **empty** body is treated as `T::default()`, so a
//! `POST` with no body is valid (e.g. `create_wallet` with no options). A present-but-invalid body
//! fails with 400. Implemented as a helper over `Bytes` rather than a custom extractor to avoid
//! `FromRequest` trait-lifetime friction.

use crate::error::ApiError;
use axum::body::Bytes;

/// Parse an optional JSON body: empty → `T::default()`, invalid → 400.
pub fn parse_optional<T>(bytes: &Bytes) -> Result<T, ApiError>
where
    T: serde::de::DeserializeOwned + Default,
{
    if bytes.is_empty() {
        return Ok(T::default());
    }
    serde_json::from_slice::<T>(bytes).map_err(|_| ApiError::BadRequest("invalid JSON body".into()))
}

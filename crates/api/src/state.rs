//! Shared application state: DB handle, master key, network, and Horizon config.

use crate::error::ApiError;
use base64::Engine;
use octo_crypto::{master_key_from_slice, MASTER_KEY_LEN};
use octo_store::Store;
use octo_wallet_core::StellarNetwork;
use std::sync::Arc;
use zeroize::Zeroizing;

/// Cloneable, shared API state.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    /// AES-256 master key used to seal/open seeds. Held zeroized.
    master_key: Zeroizing<[u8; MASTER_KEY_LEN]>,
    network: StellarNetwork,
    horizon_url: String,
    friendbot_url: Option<String>,
}

impl AppState {
    /// Build state from explicit config.
    pub fn new(
        store: Store,
        master_key: [u8; MASTER_KEY_LEN],
        network: StellarNetwork,
        horizon_url: String,
        friendbot_url: Option<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                master_key: Zeroizing::new(master_key),
                network,
                horizon_url,
                friendbot_url,
            }),
        }
    }

    /// Decode a base64 32-byte master key (from KMS/env) into raw bytes.
    pub fn decode_master_key(b64: &str) -> Result<[u8; MASTER_KEY_LEN], ApiError> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|_| ApiError::BadRequest("invalid MASTER_KEY (base64)".into()))?;
        master_key_from_slice(&raw)
            .map_err(|_| ApiError::BadRequest("MASTER_KEY must be 32 bytes".into()))
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn master_key(&self) -> &[u8; MASTER_KEY_LEN] {
        &self.inner.master_key
    }

    pub fn network(&self) -> StellarNetwork {
        self.inner.network
    }

    pub fn horizon_url(&self) -> &str {
        &self.inner.horizon_url
    }

    pub fn friendbot_url(&self) -> Option<&str> {
        self.inner.friendbot_url.as_deref()
    }
}

//! Shared application state: DB handle, master key, network, and Horizon config.

use crate::error::ApiError;
use crate::horizon::Horizon;
use base64::Engine;
use octo_crypto::{master_key_from_slice, MASTER_KEY_LEN};
use octo_store::Store;
use octo_wallet_core::StellarNetwork;
use octo_webhooks::WebhookSender;
use std::sync::Arc;
use zeroize::Zeroizing;

/// Cloneable, shared API state.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

#[derive(Clone)]
struct Inner {
    store: Store,
    /// AES-256 master key used to seal/open seeds. Held zeroized.
    master_key: Zeroizing<[u8; MASTER_KEY_LEN]>,
    network: StellarNetwork,
    horizon: Horizon,
    horizon_url: String,
    friendbot_url: Option<String>,
    /// HMAC secret for signing dashboard auth JWTs.
    jwt_secret: Vec<u8>,
    /// Fires signed webhooks (e.g. `transaction.sponsored`) to registered endpoints.
    webhooks: WebhookSender,
}

impl AppState {
    /// Build state. The JWT secret defaults to a random per-process value (fine for tests/dev;
    /// tokens won't survive a restart). Use [`AppState::with_jwt_secret`] in production.
    pub fn new(
        store: Store,
        master_key: [u8; MASTER_KEY_LEN],
        network: StellarNetwork,
        horizon_url: String,
        friendbot_url: Option<String>,
    ) -> Self {
        let mut secret = vec![0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut secret);
        Self::build(
            store,
            master_key,
            network,
            horizon_url,
            friendbot_url,
            secret,
        )
    }

    /// Set the JWT signing secret (e.g. from the `JWT_SECRET` env var) so tokens survive restarts.
    pub fn with_jwt_secret(mut self, secret: Vec<u8>) -> Self {
        // Arc is unique here in practice (called right after `new`), so rebuild the inner.
        let inner = Arc::make_mut(&mut self.inner);
        inner.jwt_secret = secret;
        self
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        store: Store,
        master_key: [u8; MASTER_KEY_LEN],
        network: StellarNetwork,
        horizon_url: String,
        friendbot_url: Option<String>,
        jwt_secret: Vec<u8>,
    ) -> Self {
        let horizon = Horizon::new(horizon_url.clone());
        let webhooks = WebhookSender::new(store.clone());
        Self {
            inner: Arc::new(Inner {
                store,
                master_key: Zeroizing::new(master_key),
                network,
                horizon,
                horizon_url,
                friendbot_url,
                jwt_secret,
                webhooks,
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

    pub fn jwt_secret(&self) -> &[u8] {
        &self.inner.jwt_secret
    }

    pub fn network(&self) -> StellarNetwork {
        self.inner.network
    }

    pub fn horizon(&self) -> &Horizon {
        &self.inner.horizon
    }

    pub fn webhooks(&self) -> &WebhookSender {
        &self.inner.webhooks
    }

    pub fn horizon_url(&self) -> &str {
        &self.inner.horizon_url
    }

    pub fn friendbot_url(&self) -> Option<&str> {
        self.inner.friendbot_url.as_deref()
    }
}

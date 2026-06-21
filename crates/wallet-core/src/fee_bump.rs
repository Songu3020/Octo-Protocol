//! Fee-bump signing for gas sponsorship: wrap a user-already-signed inner transaction in a
//! Stellar `FeeBumpTransaction` paid by the master account.
//!
//! Security posture (see `docs/threat-model.md`):
//! - Only `Payment` operations are allowed in the inner transaction. This is the anti-"signing
//!   oracle" guarantee for sponsorship: a caller cannot get the master key to pay for arbitrary
//!   on-chain effects (account merges, offers, trustline changes, ...), only plain payments.
//! - This module never signs the *inner* transaction — it must already carry its own source
//!   account's signature. Only the outer fee-bump envelope is signed here, with the master key.

use crate::error::WalletError;
use crate::signer::{keypair_from_sealed, StellarNetwork};
use octo_crypto::{SealedSeed, MASTER_KEY_LEN};
use stellar_base::amount::Stroops;
use stellar_base::operations::Operation;
use stellar_base::transaction::{FeeBumpTransaction, TransactionEnvelope};
use stellar_base::xdr::{TransactionEnvelope as XdrTransactionEnvelope, XDRDeserialize, XDRSerialize};

/// The network minimum fee per operation (stroops) — used when a wallet's sponsorship config
/// doesn't set a higher per-operation cap.
pub const MIN_FEE_PER_OP_STROOPS: i64 = 100;

/// A user-signed inner transaction to sponsor, plus the per-operation fee the master will pay.
pub struct FeeBumpRequest<'a> {
    /// Base64 `TransactionEnvelope` XDR, already signed by its own source account.
    pub inner_tx_xdr: &'a str,
    /// Fee in stroops the master pays **per operation** in the inner transaction. Must be > 0.
    pub fee_per_op_stroops: i64,
}

/// The result of building and signing a fee-bump envelope.
pub struct SignedFeeBump {
    /// Base64-encoded signed `TransactionEnvelope` (fee-bump), ready to POST to Horizon.
    pub envelope_xdr: String,
    /// Hex-encoded hash of the **inner** transaction (network-bound, stable across resubmits) —
    /// used for audit logging and duplicate-submission detection.
    pub inner_tx_hash: String,
    /// The total fee charged to the master account, in stroops.
    pub fee_stroops: i64,
    /// The `G...` master account that paid the fee.
    pub fee_source: String,
}

/// Only `Payment` operations are sponsorable today.
fn is_allowed_op(op: &Operation) -> bool {
    matches!(op, Operation::Payment(_))
}

/// Parse, validate, build, and sign a fee-bump transaction for `req.inner_tx_xdr`.
///
/// Returns:
/// - `Err(WalletError::InvalidAmount)` if `fee_per_op_stroops` is not strictly positive.
/// - `Err(WalletError::InvalidXdr)` if the XDR doesn't parse, or is itself already a fee-bump
///   (fee-bump-of-fee-bump is not a meaningful sponsorship request).
/// - `Err(WalletError::OperationNotAllowed)` if the inner transaction has no operations, or any
///   operation is outside the sponsor allowlist (`Payment` only).
pub fn sign_fee_bump(
    master_key: &[u8; MASTER_KEY_LEN],
    sealed: &SealedSeed,
    network: StellarNetwork,
    account_index: u32,
    req: &FeeBumpRequest<'_>,
) -> Result<SignedFeeBump, WalletError> {
    if req.fee_per_op_stroops <= 0 {
        return Err(WalletError::InvalidAmount);
    }

    let raw = XdrTransactionEnvelope::from_xdr_base64(req.inner_tx_xdr)
        .map_err(|_| WalletError::InvalidXdr)?;
    let envelope = TransactionEnvelope::from_xdr(&raw).map_err(|_| WalletError::InvalidXdr)?;
    let inner = match envelope {
        TransactionEnvelope::Transaction(tx) => tx,
        TransactionEnvelope::FeeBumpTransaction(_) => return Err(WalletError::InvalidXdr),
    };

    let ops = inner.operations();
    if ops.is_empty() || !ops.iter().all(is_allowed_op) {
        return Err(WalletError::OperationNotAllowed);
    }

    let op_count = i64::try_from(ops.len()).map_err(|_| WalletError::OperationNotAllowed)?;
    let fee_stroops = req
        .fee_per_op_stroops
        .checked_mul(op_count)
        .ok_or(WalletError::InvalidAmount)?;

    let keypair = keypair_from_sealed(master_key, sealed, network, account_index)?;
    let fee_source_pk = keypair.public_key();
    let fee_source = fee_source_pk.account_id();

    let net = network.to_base();
    let inner_tx_hash = inner.hash(&net).map_err(|_| WalletError::Signing)?;

    let mut fee_bump = FeeBumpTransaction::new(fee_source_pk.into(), Stroops::new(fee_stroops), inner);
    fee_bump
        .sign(keypair.as_ref(), &net)
        .map_err(|_| WalletError::Signing)?;

    let envelope_xdr = fee_bump
        .into_envelope()
        .xdr_base64()
        .map_err(|_| WalletError::Signing)?;

    Ok(SignedFeeBump {
        envelope_xdr,
        inner_tx_hash: hex::encode(inner_tx_hash),
        fee_stroops,
        fee_source,
    })
}

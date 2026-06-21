//! Error type for wallet-core.
//!
//! Like [`octo_crypto::CryptoError`], variants avoid carrying secret material. They describe the
//! *kind* of failure (bad input, derivation, signing) without echoing keys, seeds, or amounts.

use thiserror::Error;

/// Errors returned by wallet-core operations.
#[derive(Debug, Error)]
pub enum WalletError {
    /// The supplied BIP39 mnemonic phrase was invalid.
    #[error("invalid mnemonic phrase")]
    InvalidMnemonic,

    /// A derivation path component or index was invalid.
    #[error("invalid derivation path")]
    InvalidDerivationPath,

    /// Failed to construct a Stellar keypair from the derived seed bytes.
    #[error("key derivation failed")]
    KeyDerivation,

    /// An address string (G... or M...) could not be parsed.
    #[error("invalid Stellar address")]
    InvalidAddress,

    /// A requested amount was out of range (must be a positive number of stroops).
    #[error("invalid amount")]
    InvalidAmount,

    /// Building or signing the transaction failed.
    #[error("transaction signing failed")]
    Signing,

    /// Decrypting the sealed seed failed (wrong key/context or tampered record).
    #[error("seed decryption failed")]
    SeedDecryption,

    /// The supplied transaction XDR could not be parsed, or used an unsupported envelope shape
    /// (e.g. a fee-bump wrapping another fee-bump).
    #[error("invalid transaction XDR")]
    InvalidXdr,

    /// An operation in the inner transaction is outside the sponsor allowlist.
    #[error("operation type not allowed for sponsorship")]
    OperationNotAllowed,
}

impl From<octo_crypto::CryptoError> for WalletError {
    fn from(_: octo_crypto::CryptoError) -> Self {
        // Collapse all crypto failures to a single coarse variant — do not leak which.
        WalletError::SeedDecryption
    }
}

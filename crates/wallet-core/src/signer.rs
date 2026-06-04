//! The signing path: open a sealed seed, derive the master key, build a **payment** transaction,
//! sign it, and zeroize secrets.
//!
//! Security posture (see `docs/threat-model.md`):
//! - This module only ever builds octo's own **Payment** operations. It does **not** accept or
//!   sign caller-supplied raw XDR, so it cannot be used as a "sign anything" oracle.
//! - Amounts are integer **stroops** (`i64`), validated to be strictly positive.
//! - The network (testnet/mainnet) is always explicit — there is no ambient default that could
//!   cause a testnet-intended signature to be valid on mainnet.
//! - The decrypted seed and the derived keypair live only for the duration of `sign_payment` and
//!   are zeroized on drop.

use crate::address::is_valid_account;
use crate::derive::WalletSeed;
use crate::error::WalletError;
use octo_crypto::{open, SealedSeed, MASTER_KEY_LEN};
use stellar_base::amount::Stroops;
use stellar_base::asset::Asset;
use stellar_base::crypto::{DalekKeyPair, MuxedEd25519PublicKey, PublicKey};
use stellar_base::memo::Memo;
use stellar_base::network::Network;
use stellar_base::operations::Operation;
use stellar_base::transaction::{Transaction, MIN_BASE_FEE};
use stellar_base::xdr::XDRSerialize;

/// Which Stellar network a signature targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StellarNetwork {
    /// Public (mainnet) network.
    Public,
    /// Test network.
    Testnet,
}

impl StellarNetwork {
    fn to_base(self) -> Network {
        match self {
            StellarNetwork::Public => Network::new_public(),
            StellarNetwork::Testnet => Network::new_test(),
        }
    }

    /// The crypto context string bound into seed encryption for this network.
    pub fn crypto_context(self) -> &'static [u8] {
        match self {
            StellarNetwork::Public => b"octo:mainnet",
            StellarNetwork::Testnet => b"octo:testnet",
        }
    }
}

/// A single payment to build and sign from the master account.
pub struct PaymentRequest<'a> {
    /// Destination account (`G...`) or muxed (`M...`) address.
    pub destination: &'a str,
    /// Amount in **stroops** (1 XLM = 10_000_000 stroops). Must be > 0.
    pub stroops: i64,
    /// `None` => native XLM. `Some((code, issuer_g))` => a credit asset.
    pub asset: Option<(&'a str, &'a str)>,
    /// Optional numeric memo (used for the `G...`+memo deposit-return convention).
    pub memo_id: Option<u64>,
    /// The master account's current sequence number (fetched from Horizon by the caller).
    pub sequence: i64,
}

/// The result of signing: the base64 XDR envelope to submit, plus the master account it was
/// signed for. (The transaction hash is computed by the caller/Horizon on submit.)
pub struct SignedPayment {
    /// Base64-encoded signed `TransactionEnvelope`, ready to POST to Horizon.
    pub envelope_xdr: String,
    /// The `G...` master account that sourced and signed this transaction.
    pub source_account: String,
}

/// Open a sealed seed for `network`, derive Stellar account `account_index`, and return its
/// `DalekKeyPair`. The decrypted seed is zeroized as it leaves scope.
fn keypair_from_sealed(
    master_key: &[u8; MASTER_KEY_LEN],
    sealed: &SealedSeed,
    network: StellarNetwork,
    account_index: u32,
) -> Result<DalekKeyPair, WalletError> {
    let seed_bytes = open(master_key, sealed, network.crypto_context())?;
    let seed = WalletSeed::from_bytes(seed_bytes.to_vec());
    let secret = seed.derive_ed25519_secret(account_index);
    // stellar-base builds the ed25519 keypair from the 32-byte secret seed.
    DalekKeyPair::from_seed_bytes(secret.as_ref()).map_err(|_| WalletError::KeyDerivation)
}

/// Derive just the `G...` account id for `account_index` from a sealed seed (no signing).
pub fn account_id_from_sealed(
    master_key: &[u8; MASTER_KEY_LEN],
    sealed: &SealedSeed,
    network: StellarNetwork,
    account_index: u32,
) -> Result<String, WalletError> {
    let kp = keypair_from_sealed(master_key, sealed, network, account_index)?;
    Ok(kp.public_key().account_id())
}

/// Build and sign a payment from the master account (`account_index`, normally 0).
///
/// Only a Payment operation is ever constructed — no other operation type can be produced by this
/// function, which is the core anti-"signing-oracle" guarantee.
pub fn sign_payment(
    master_key: &[u8; MASTER_KEY_LEN],
    sealed: &SealedSeed,
    network: StellarNetwork,
    account_index: u32,
    req: &PaymentRequest<'_>,
) -> Result<SignedPayment, WalletError> {
    if req.stroops <= 0 {
        return Err(WalletError::InvalidAmount);
    }

    let keypair = keypair_from_sealed(master_key, sealed, network, account_index)?;
    let source = keypair.public_key();
    let source_account = source.account_id();

    // Resolve the destination (accept either G... or M...).
    let destination = parse_destination(req.destination)?;

    // Resolve the asset (native XLM or a validated credit asset).
    let asset = match req.asset {
        None => Asset::new_native(),
        Some((code, issuer)) => {
            if !is_valid_account(issuer) {
                return Err(WalletError::InvalidAddress);
            }
            let issuer_pk =
                PublicKey::from_account_id(issuer).map_err(|_| WalletError::InvalidAddress)?;
            Asset::new_credit(code, issuer_pk).map_err(|_| WalletError::InvalidAddress)?
        }
    };

    let payment = Operation::new_payment()
        .with_destination(destination)
        .with_amount(Stroops::new(req.stroops))
        .map_err(|_| WalletError::InvalidAmount)?
        .with_asset(asset)
        .build()
        .map_err(|_| WalletError::Signing)?;

    let mut builder = Transaction::builder(source, req.sequence, MIN_BASE_FEE);
    if let Some(id) = req.memo_id {
        builder = builder.with_memo(Memo::new_id(id));
    }
    let mut tx = builder
        .add_operation(payment)
        .into_transaction()
        .map_err(|_| WalletError::Signing)?;

    // DalekKeyPair derefs to the inner KeyPair, which is what sign() accepts.
    tx.sign(keypair.as_ref(), &network.to_base())
        .map_err(|_| WalletError::Signing)?;

    let envelope_xdr = tx
        .into_envelope()
        .xdr_base64()
        .map_err(|_| WalletError::Signing)?;

    Ok(SignedPayment {
        envelope_xdr,
        source_account,
    })
}

/// Parse a destination that may be a `G...` account or an `M...` muxed address.
fn parse_destination(dest: &str) -> Result<stellar_base::crypto::MuxedAccount, WalletError> {
    if let Ok(mux) = MuxedEd25519PublicKey::from_account_id(dest) {
        return Ok(mux.into());
    }
    let pk = PublicKey::from_account_id(dest).map_err(|_| WalletError::InvalidAddress)?;
    Ok(pk.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use octo_crypto::seal;
    use stellar_base::xdr::XDRDeserialize;

    const VECTOR_MNEMONIC: &str =
        "illness spike retreat truth genius clock brain pass fit cave bargain toe";
    const MASTER_ACCOUNT_0: &str = "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6";
    // A valid destination: account index 1 derived from the same vector seed.
    const DEST: &str = "GBAW5XGWORWVFE2XTJYDTLDHXTY2Q2MO73HYCGB3XMFMQ562Q2W2GJQX";

    fn sealed_vector_seed(net: StellarNetwork) -> ([u8; 32], SealedSeed) {
        let mk = [7u8; 32];
        // The raw 64-byte BIP39 seed for the SEP-0005 vector mnemonic, sealed for `net`.
        let bytes = bip39::Seed::new(
            &bip39::Mnemonic::from_phrase(VECTOR_MNEMONIC, bip39::Language::English).unwrap(),
            "",
        )
        .as_bytes()
        .to_vec();
        let sealed = seal(&mk, &bytes, net.crypto_context()).unwrap();
        (mk, sealed)
    }

    #[test]
    fn account_id_from_sealed_matches_vector() {
        let (mk, sealed) = sealed_vector_seed(StellarNetwork::Testnet);
        let acct = account_id_from_sealed(&mk, &sealed, StellarNetwork::Testnet, 0).unwrap();
        assert_eq!(acct, MASTER_ACCOUNT_0);
    }

    #[test]
    fn signs_native_payment_and_produces_valid_envelope() {
        let (mk, sealed) = sealed_vector_seed(StellarNetwork::Testnet);
        let req = PaymentRequest {
            destination: DEST,
            stroops: 10_000_000, // 1 XLM
            asset: None,
            memo_id: None,
            sequence: 1,
        };
        let signed = sign_payment(&mk, &sealed, StellarNetwork::Testnet, 0, &req).unwrap();
        assert_eq!(signed.source_account, MASTER_ACCOUNT_0);
        // The envelope must be valid, signed XDR that round-trips through the parser.
        let env = stellar_base::xdr::TransactionEnvelope::from_xdr_base64(&signed.envelope_xdr)
            .expect("signed envelope must be valid XDR");
        // It must carry exactly one signature.
        match env {
            stellar_base::xdr::TransactionEnvelope::Tx(e) => {
                assert_eq!(e.signatures.len(), 1, "must be signed once");
            }
            _ => panic!("unexpected envelope variant"),
        }
    }

    #[test]
    fn rejects_non_positive_amount() {
        let (mk, sealed) = sealed_vector_seed(StellarNetwork::Testnet);
        for bad in [0i64, -1, -10_000_000] {
            let req = PaymentRequest {
                destination: DEST,
                stroops: bad,
                asset: None,
                memo_id: None,
                sequence: 1,
            };
            assert!(matches!(
                sign_payment(&mk, &sealed, StellarNetwork::Testnet, 0, &req),
                Err(WalletError::InvalidAmount)
            ));
        }
    }

    #[test]
    fn rejects_bad_destination() {
        let (mk, sealed) = sealed_vector_seed(StellarNetwork::Testnet);
        let req = PaymentRequest {
            destination: "not-an-address",
            stroops: 1,
            asset: None,
            memo_id: None,
            sequence: 1,
        };
        assert!(matches!(
            sign_payment(&mk, &sealed, StellarNetwork::Testnet, 0, &req),
            Err(WalletError::InvalidAddress)
        ));
    }

    #[test]
    fn wrong_network_context_cannot_open_seed() {
        // Seed sealed for mainnet; signing as testnet must fail to decrypt (AAD/context mismatch).
        let (mk, sealed) = sealed_vector_seed(StellarNetwork::Public);
        let req = PaymentRequest {
            destination: DEST,
            stroops: 1,
            asset: None,
            memo_id: None,
            sequence: 1,
        };
        assert!(matches!(
            sign_payment(&mk, &sealed, StellarNetwork::Testnet, 0, &req),
            Err(WalletError::SeedDecryption)
        ));
    }

    #[test]
    fn signs_payment_to_muxed_destination() {
        let (mk, sealed) = sealed_vector_seed(StellarNetwork::Testnet);
        let muxed = crate::address::encode_muxed(DEST, 99).unwrap();
        let req = PaymentRequest {
            destination: &muxed,
            stroops: 5,
            asset: None,
            memo_id: None,
            sequence: 2,
        };
        let signed = sign_payment(&mk, &sealed, StellarNetwork::Testnet, 0, &req).unwrap();
        assert!(!signed.envelope_xdr.is_empty());
    }
}

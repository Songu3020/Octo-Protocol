//! Customer deposit addresses: muxed (`M...`) primary + `G...`+memo fallback.
//!
//! A muxed account is the base account's ed25519 key plus a 64-bit id. octo gives each customer a
//! unique id; funds sent to their `M...` land in the single base account and carry the id, so we
//! attribute deposits with no sweep and no per-user reserve. For senders that can't send to `M...`
//! (some exchanges), the same id is exposed as a numeric **memo** against the base `G...`.
//!
//! See `docs/deposit-model.md`.

use crate::error::WalletError;
use serde::{Deserialize, Serialize};
use stellar_strkey::ed25519::{MuxedAccount, PublicKey};

/// Both forms of a customer deposit address, derived from one base account + id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositAddress {
    /// The muxed address (`M...`) — the default form handed to customers.
    pub muxed_address: String,
    /// The base account (`G...`) — the fallback destination for `G...`+memo senders.
    pub base_address: String,
    /// The numeric id encoded in the muxed address; also the memo id for the fallback path.
    pub memo_id: u64,
}

/// Build both address forms for `base_account` (`G...`) and `id`.
///
/// `base_account` must be a valid ed25519 public-key strkey (`G...`).
pub fn deposit_address(base_account: &str, id: u64) -> Result<DepositAddress, WalletError> {
    let pk = PublicKey::from_string(base_account).map_err(|_| WalletError::InvalidAddress)?;
    let muxed = MuxedAccount { ed25519: pk.0, id };
    Ok(DepositAddress {
        // stellar-strkey's inherent to_string() returns a heapless::String; `format!` via Display
        // yields a std String.
        muxed_address: format!("{muxed}"),
        base_address: base_account.to_string(),
        memo_id: id,
    })
}

/// Encode a base account (`G...`) + id into a muxed address (`M...`).
pub fn encode_muxed(base_account: &str, id: u64) -> Result<String, WalletError> {
    let pk = PublicKey::from_string(base_account).map_err(|_| WalletError::InvalidAddress)?;
    let muxed = MuxedAccount { ed25519: pk.0, id };
    Ok(format!("{muxed}"))
}

/// The base account + id recovered from a muxed address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedMuxed {
    /// The base ed25519 public key bytes.
    pub ed25519: [u8; 32],
    /// The 64-bit id (customer id / memo id).
    pub id: u64,
}

impl DecodedMuxed {
    /// The base account as a `G...` strkey.
    pub fn base_account(&self) -> String {
        let pk = PublicKey(self.ed25519);
        format!("{pk}")
    }
}

/// Decode a muxed address (`M...`) back into its base account and id.
pub fn decode_muxed(muxed_address: &str) -> Result<DecodedMuxed, WalletError> {
    let mux = MuxedAccount::from_string(muxed_address).map_err(|_| WalletError::InvalidAddress)?;
    Ok(DecodedMuxed {
        ed25519: mux.ed25519,
        id: mux.id,
    })
}

/// Validate that a string is a well-formed base account address (`G...`).
pub fn is_valid_account(address: &str) -> bool {
    PublicKey::from_string(address).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A valid testnet/mainnet-format account (the SEP-0005 Test 1 account 0).
    const BASE: &str = "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6";

    #[test]
    fn deposit_address_has_both_forms() {
        let addr = deposit_address(BASE, 42).unwrap();
        assert!(addr.muxed_address.starts_with('M'));
        assert_eq!(addr.base_address, BASE);
        assert_eq!(addr.memo_id, 42);
    }

    #[test]
    fn muxed_roundtrip() {
        let m = encode_muxed(BASE, 1234567890).unwrap();
        assert!(m.starts_with('M'));
        let decoded = decode_muxed(&m).unwrap();
        assert_eq!(decoded.id, 1234567890);
        assert_eq!(decoded.base_account(), BASE);
    }

    #[test]
    fn different_ids_differ_but_share_base() {
        let a = decode_muxed(&encode_muxed(BASE, 1).unwrap()).unwrap();
        let b = decode_muxed(&encode_muxed(BASE, 2).unwrap()).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(a.base_account(), b.base_account());
        assert_eq!(a.base_account(), BASE);
    }

    #[test]
    fn id_zero_and_max_roundtrip() {
        for id in [0u64, u64::MAX] {
            let decoded = decode_muxed(&encode_muxed(BASE, id).unwrap()).unwrap();
            assert_eq!(decoded.id, id);
        }
    }

    #[test]
    fn rejects_invalid_base_account() {
        assert!(matches!(
            encode_muxed("not-an-address", 1),
            Err(WalletError::InvalidAddress)
        ));
        assert!(!is_valid_account("not-an-address"));
        assert!(is_valid_account(BASE));
    }

    #[test]
    fn rejects_invalid_muxed() {
        assert!(matches!(
            decode_muxed("MABADDRESS"),
            Err(WalletError::InvalidAddress)
        ));
        // A G... address is not a muxed address.
        assert!(matches!(
            decode_muxed(BASE),
            Err(WalletError::InvalidAddress)
        ));
    }
}

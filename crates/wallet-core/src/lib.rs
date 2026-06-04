//! SEP-0005 (SLIP-0010 ed25519) derivation, muxed address encode/decode, and Stellar
//! transaction signing. This is the only crate that handles secret key material; decrypted
//! seeds and derived keys are zeroized after use.
//!
//! Implemented in Step 4 of the project plan.
#![forbid(unsafe_code)]
// Secret-handling crate: a panic could surface key material in a backtrace, and lossy/sign
// conversions on amounts are bugs. Deny them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

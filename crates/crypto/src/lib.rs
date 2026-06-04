//! AES-256-GCM seal/open of the HD seed at rest.
//!
//! Implemented in Step 3 of the project plan.
#![forbid(unsafe_code)]
// Secret-handling crate: a panic could surface key material in a backtrace, and lossy/sign
// conversions on amounts are bugs. Deny them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![deny(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

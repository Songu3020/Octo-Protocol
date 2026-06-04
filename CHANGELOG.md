# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Workspace scaffold: `crypto`, `wallet-core`, `store`, `webhooks`, `ingest`, `api` crates and
  the `server` binary.
- Repository tooling: CI (fmt, clippy, test, cargo-deny), `justfile`, `docker-compose` for local
  Postgres, contribution/security docs, MIT license.
- Pinned dependency set verified against the official SEP-0005 test vectors and Stellar muxed
  (`M...`) address round-trips.
- `octo-crypto`: AES-256-GCM seal/open of the HD seed at rest, with HKDF-SHA256 per-record
  subkeys, AAD context binding (records are bound to e.g. `octo:mainnet`), and zeroized
  plaintext/keys. Covered by 10 tests including tamper, wrong-key, and wrong-context negatives.
- Security: `docs/threat-model.md`; deny `unwrap`/`expect`/`panic` and lossy casts in
  secret-handling crates; CI adds cargo-audit and gitleaks.
- `octo-wallet-core`: SEP-0005 ed25519 derivation (`m/44'/148'/index'`, matches the official
  vector), muxed (`M...`) + `G...`+memo deposit addresses, and a payment signing path that opens
  the sealed seed, derives the key, builds **only** a Payment op (no raw-XDR signing oracle),
  signs, and zeroizes. 17 tests including non-positive-amount, bad-destination, and
  wrong-network-decryption negatives.

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
- `octo-store`: Postgres schema (wallets, addresses, transactions, withdrawals, webhook
  endpoints/deliveries, ingest cursor) and a sqlx `Store` API. Idempotent deposit recording
  (unique `(tx_hash, operation_index)` → anti double-credit), idempotency-keyed withdrawals,
  atomic row-locked muxed-id allocation, and a durable ingest cursor. Amounts are integer
  stroops. 6 integration tests against Postgres.
- `octo-wallet-core`: `provision_wallet`/`import_wallet` tie mnemonic generation, account
  derivation, and sealing into one call; `StellarNetwork::{as_str,parse}`.
- `octo-api`: axum REST service with `POST /v1/wallets` (generate → seal → store, returns the
  `G...` address + one-time recovery mnemonic), `GET /v1/wallets/:id`, and
  `POST|GET /v1/wallets/:id/addresses` (returns both the muxed `M...` and the `G...`+memo_id
  fallback). Standard `{statusCode,message,data}` envelope; errors never leak internals. 4
  integration tests drive the real router (incl. a check that the stored seed is ciphertext, not
  the plaintext mnemonic).

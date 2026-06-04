# octo Threat Model

> **Honest scope.** No system is "safe against all hacks." This document enumerates the attack
> classes that actually apply to octo's architecture, the concrete defense for each, and the step
> that enforces it. It is a living document — every new feature must say which of these it touches.

## What octo is (for threat-modeling)

octo is a **custodial backend service** that holds one HD seed per network (encrypted at rest),
derives Stellar keys on demand, and signs Stellar transactions. It is **not** a smart-contract
system. Therefore the famous web3 exploit classes that assume on-chain contracts **do not apply**:

| Classic web3 hack | Applies to octo? | Why |
|---|---|---|
| Reentrancy, delegatecall, proxy bugs | ❌ | No smart contracts. We send native Stellar payment ops. |
| Flash-loan / oracle manipulation | ❌ | We don't price or lend. |
| Bridge / cross-chain exploits | ❌ (MVP) | No bridging in the MVP. |
| `tx.origin`, integer overflow in Solidity | ❌ | Not Solidity. Rust + checked arithmetic. |
| Approval / `permit` phishing | ❌ | No ERC-20 allowances on Stellar. |

octo's **real** threat surface is **key custody, the signing path, deposit/withdrawal accounting,
and ordinary web2 backend security.** Those are where custody services actually get drained.

---

## Threat classes that DO apply, and our defenses

### A. Key & seed compromise  *(highest severity — 44% of 2024 crypto theft was key compromise)*
| Threat | Defense | Step |
|---|---|---|
| Seed stolen from DB dump / backup | Seed stored **AES-256-GCM encrypted**, random nonce+salt; master key from KMS/secret-manager, never in the DB or repo | 3, 5 |
| Seed/keys leaked via logs, crash dumps, swap | Secrets live only in `wallet-core`; `Zeroizing` wrappers wipe seed & derived keys on drop; `Debug` never prints secret bytes; no `unwrap()` that could panic-print secrets | 3, 4 |
| Master key compromise | KMS-held key, rotation path documented; defense-in-depth so DB-only compromise is insufficient | 3 (design), later (KMS) |
| Weak randomness in key/nonce generation | Use `OsRng` (CSPRNG) only; never `rand::thread_rng` seeded predictably for key material; test that two seals of same plaintext differ | 3, 4 |
| Derived key reused across contexts | One derivation path per account; keys are ephemeral and zeroized | 4 |

### B. Signing-path abuse  *(a signing service is a "sign anything" oracle if unguarded)*
| Threat | Defense | Step |
|---|---|---|
| Attacker gets octo to sign an arbitrary/malicious tx | octo only builds txs from **its own** validated intents (payment ops to a validated destination); it does **not** sign caller-supplied raw XDR in the MVP. Any future sign-only endpoint validates op-by-op against an allowlist | 10 |
| Confused-deputy: API caller withdraws others' funds | Every withdrawal is authorized against the **wallet's API key / tenant**; destination + amount validated server-side; no client-controlled source account | 6, 10 |
| Fee/op injection (sponsored-reserve, set-options, merge-account) | Whitelist allowed operation types (Payment only in MVP); reject `ACCOUNT_MERGE`, `SET_OPTIONS`, `CHANGE_TRUST` from any caller-influenced path | 10 |

### C. Deposit accounting attacks  *(how exchanges actually get double-spent)*
| Threat | Defense | Step |
|---|---|---|
| **Double-credit on failed/reorged tx** (the Mt. Gox class) | Only credit deposits with `successful == true` from Horizon; key off the **immutable tx hash + operation id**; idempotent insert (unique constraint) so replays can't double-credit | 8 |
| **Memo-less / wrong-memo deposit** misattribution | Attribute strictly by muxed id **or** a valid numeric memo id that maps to a known address; unmatched deposits go to a **quarantine/unattributed** state, never auto-credited to a guess | 8 |
| Replayed Horizon events | Cursor is monotonic + dedup by tx hash; reprocessing the same payment is a no-op | 8 |
| Spoofed asset / fake token deposit | Credit only **whitelisted assets** (issuer + code must match an enabled asset); ignore unknown trustlines/tokens | 8, later (asset mgmt) |
| Claimable-balance side-channel | Treat claimable balances explicitly; do not credit until claimed into the master under our control | 8 (documented), later |
| Dust / griefing to inflate accounting | Minimum-amount thresholds; amounts stored as exact integers (stroops), never floats | 8 |

### D. Withdrawal / payout attacks
| Threat | Defense | Step |
|---|---|---|
| Double-withdraw via retried request | **Idempotency key** per withdrawal intent; state machine (`pending→submitted→confirmed/failed`) with DB uniqueness; never re-sign a settled intent | 10 |
| Race / TOCTOU on balance | Balance checks + intent creation in a single DB transaction with row locking | 10 |
| Amount precision bug (float rounding) | All amounts are integer **stroops** end-to-end; convert at the edge only | 4, 10 |
| Destination tampering in transit | TLS everywhere; destination validated (valid strkey) and echoed back in the signed tx the caller can verify | 10 |

### E. Web2 backend surface  *(the unglamorous majority of real breaches)*
| Threat | Defense | Step |
|---|---|---|
| SQL injection | `sqlx` parameterized queries only; no string-built SQL | 5 |
| AuthN/AuthZ bypass | API-key auth per tenant; constant-time key comparison; per-route authorization | 6 |
| SSRF via webhook/Horizon URLs | Validate/allowlist outbound URLs; block internal/metadata IP ranges for webhook targets | 9 |
| Webhook forgery / tampering | Outbound webhooks **HMAC-SHA256 signed**; consumers verify. Inbound (if any) verified | 9 |
| Replay of API requests | TLS + idempotency keys on mutating endpoints | 10 |
| Secrets in env/CI logs | `.env` git-ignored; CI uses masked secrets; no secret echoed in logs | 2 ✓ |
| Dependency supply-chain (malicious/yanked crate) | `cargo-deny` (advisories + licenses + yanked) in CI; pinned `Cargo.lock`; MSRV-locked tree | 2 ✓ |
| DoS / resource exhaustion | Request limits, timeouts, connection pool caps; pagination bounds | 6, 11 |
| Information leak in errors | Typed errors; no internal detail or secret in API responses | 5, 6 |

### F. Operational / process
| Threat | Defense | Step |
|---|---|---|
| Unsafe Rust memory bugs | `#![forbid(unsafe_code)]` in every crate (already set) | 2 ✓ |
| Panics that crash or leak | `clippy::unwrap_used`/`expect_used` denied in `wallet-core`; errors propagated | 3, 4 |
| No audit trail | Append-only transaction + webhook-delivery logs | 5, 9 |
| Disaster recovery | Seed is recoverable from the BIP39 mnemonic held out-of-band; documented | later |

---

## Enforced continuously (CI gates)
- `cargo-deny` — advisories, yanked crates, license policy.
- `cargo clippy -D warnings` — plus secret-safety lints in `wallet-core`/`crypto`.
- Tests must include **negative** cases (tamper, replay, wrong-asset, double-spend).
- (Planned) `cargo audit`, secret-scanning, and a fuzz target for derivation/decode.

## Known limitations (MVP, by design — see plan "out of scope")
- Single encrypted seed (not yet MPC/HSM). Upgrade path documented in [architecture.md](architecture.md).
- Hot signing service (online key). Mitigated by encryption + zeroize + op allowlist; cold/MPC is a later phase.

> If you add a feature, add its row(s) above and the test that proves the defense.

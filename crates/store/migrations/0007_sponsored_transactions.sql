-- Sponsored transactions: an immutable audit trail of every fee-bump the master wallet sponsors,
-- and the source of truth for atomic daily-budget enforcement (see `Store::
-- record_sponsored_tx_if_budget_available`).
--
-- The unique index on `inner_tx_hash` prevents double-sponsoring the same user transaction at the
-- DB level. All monetary fields are i64 stroops (never floats).

CREATE TABLE sponsored_transactions (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id     UUID NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    inner_tx_hash TEXT NOT NULL,            -- hash of the user's inner transaction
    fee_stroops   BIGINT NOT NULL,          -- fee reserved/charged against the daily budget
    status        TEXT NOT NULL DEFAULT 'pending',  -- pending | confirmed | failed
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One sponsorship per inner transaction — anti double-sponsoring.
CREATE UNIQUE INDEX idx_sponsored_inner_tx_hash ON sponsored_transactions(inner_tx_hash);

-- Backs the per-wallet, per-day budget query.
CREATE INDEX idx_sponsored_wallet_time ON sponsored_transactions(wallet_id, created_at);

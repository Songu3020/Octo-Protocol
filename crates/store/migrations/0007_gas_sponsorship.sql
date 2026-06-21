-- Gas sponsorship: per-wallet settings (enable/disable, spend controls) and a record of each
-- sponsored transaction outcome, used for the audit trail, daily budget enforcement, and
-- duplicate-submission prevention.

CREATE TABLE gas_sponsorship_configs (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id            UUID NOT NULL UNIQUE REFERENCES wallets(id) ON DELETE CASCADE,
    enabled              BOOLEAN NOT NULL DEFAULT false,
    fee_cap_stroops      BIGINT NOT NULL DEFAULT 0,
    daily_budget_stroops BIGINT NOT NULL DEFAULT 0,
    spent_today_stroops  BIGINT NOT NULL DEFAULT 0,
    budget_date          DATE NOT NULL DEFAULT CURRENT_DATE,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sponsored_transactions (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id        UUID NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    inner_tx_hash    TEXT NOT NULL,
    fee_stroops      BIGINT NOT NULL,
    status           TEXT NOT NULL,
    stellar_tx_hash  TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (wallet_id, inner_tx_hash)
);

CREATE INDEX idx_sponsored_tx_wallet_time ON sponsored_transactions(wallet_id, created_at DESC);

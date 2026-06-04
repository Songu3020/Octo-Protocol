-- octo initial schema.
--
-- Design notes (see docs/threat-model.md):
--  * All on-chain amounts are stored as BIGINT stroops (1 XLM = 10_000_000 stroops). Never float.
--  * Deposits are deduplicated on the immutable (stellar_tx_hash, operation_index) so a replayed
--    or reorged Horizon event cannot double-credit.
--  * Withdrawals carry a client idempotency key so a retried request cannot double-spend.
--  * Seeds are stored only as ciphertext (octo-crypto SealedSeed): ciphertext + nonce + salt.

CREATE EXTENSION IF NOT EXISTS "pgcrypto";  -- for gen_random_uuid()

-- ---------------------------------------------------------------------------
-- wallets: one master wallet per network. Holds the AES-256-GCM-sealed HD seed.
-- ---------------------------------------------------------------------------
CREATE TABLE wallets (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    network                  TEXT NOT NULL CHECK (network IN ('mainnet', 'testnet')),
    -- The master account's G... address (account index 0).
    stellar_account_g        TEXT NOT NULL,
    -- Sealed seed (octo-crypto): ciphertext includes the GCM tag.
    sealed_ciphertext        BYTEA NOT NULL,
    sealed_nonce             BYTEA NOT NULL,
    sealed_salt              BYTEA NOT NULL,
    -- Monotonic counter for the next customer muxed id (assigned off-chain).
    next_muxed_id            BIGINT NOT NULL DEFAULT 1,
    label                    TEXT,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (stellar_account_g)
);

-- ---------------------------------------------------------------------------
-- addresses: per-customer deposit addresses. Cheap, off-chain rows.
-- A muxed id is unique within a wallet; the M... and G...+memo forms are derived from it.
-- ---------------------------------------------------------------------------
CREATE TABLE addresses (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id       UUID NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    -- The 64-bit muxed id. Stored as BIGINT; values are app-assigned and always >= 1, so they fit
    -- the signed range comfortably for the MVP.
    muxed_id        BIGINT NOT NULL CHECK (muxed_id >= 0),
    -- Cached derived forms for convenience / lookups.
    muxed_address   TEXT NOT NULL,
    -- Caller-supplied reference for their own user (opaque to octo).
    customer_ref    TEXT,
    -- Arbitrary JSON echoed back in webhooks for reconciliation.
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (wallet_id, muxed_id),
    UNIQUE (muxed_address)
);

CREATE INDEX idx_addresses_wallet ON addresses(wallet_id);

-- ---------------------------------------------------------------------------
-- transactions: deposits and withdrawals (append-only ledger of on-chain activity).
-- ---------------------------------------------------------------------------
CREATE TABLE transactions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id           UUID NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    -- Null for deposits that could not be attributed to a customer (quarantine).
    address_id          UUID REFERENCES addresses(id) ON DELETE SET NULL,
    direction           TEXT NOT NULL CHECK (direction IN ('deposit', 'withdrawal')),
    -- 'native' for XLM, otherwise the asset code; issuer is the G... (null for native).
    asset_code          TEXT NOT NULL,
    asset_issuer        TEXT,
    amount_stroops      BIGINT NOT NULL CHECK (amount_stroops > 0),
    source_account      TEXT,
    destination_account TEXT,
    -- Immutable on-chain identifiers used for idempotent dedup.
    stellar_tx_hash     TEXT,
    operation_index     INTEGER,
    ledger              BIGINT,
    memo_id             BIGINT,
    status              TEXT NOT NULL DEFAULT 'confirmed'
                          CHECK (status IN ('pending', 'confirmed', 'failed')),
    reference           TEXT,
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A confirmed on-chain operation is unique: this is the anti-double-credit guard. Partial unique
-- index so rows without a hash (e.g. pending intents) are not constrained.
CREATE UNIQUE INDEX uq_tx_onchain
    ON transactions (stellar_tx_hash, operation_index)
    WHERE stellar_tx_hash IS NOT NULL;

CREATE INDEX idx_tx_wallet ON transactions(wallet_id);
CREATE INDEX idx_tx_address ON transactions(address_id);

-- ---------------------------------------------------------------------------
-- withdrawals: payout intents with a state machine + idempotency key.
-- ---------------------------------------------------------------------------
CREATE TABLE withdrawals (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id           UUID NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    idempotency_key     TEXT NOT NULL,
    destination_account TEXT NOT NULL,
    asset_code          TEXT NOT NULL,
    asset_issuer        TEXT,
    amount_stroops      BIGINT NOT NULL CHECK (amount_stroops > 0),
    memo_id             BIGINT,
    status              TEXT NOT NULL DEFAULT 'pending'
                          CHECK (status IN ('pending', 'submitted', 'confirmed', 'failed')),
    stellar_tx_hash     TEXT,
    error               TEXT,
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A retried request with the same key must not create a second payout.
    UNIQUE (wallet_id, idempotency_key)
);

CREATE INDEX idx_withdrawals_wallet ON withdrawals(wallet_id);

-- ---------------------------------------------------------------------------
-- webhook_endpoints: where to deliver events, and the HMAC signing secret.
-- ---------------------------------------------------------------------------
CREATE TABLE webhook_endpoints (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id   UUID NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    url         TEXT NOT NULL,
    secret      TEXT NOT NULL,
    active      BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_webhook_endpoints_wallet ON webhook_endpoints(wallet_id);

-- ---------------------------------------------------------------------------
-- webhook_deliveries: per-attempt delivery log (audit trail).
-- ---------------------------------------------------------------------------
CREATE TABLE webhook_deliveries (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    endpoint_id   UUID NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
    event_type    TEXT NOT NULL,
    payload       JSONB NOT NULL,
    status        TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'delivered', 'failed')),
    attempts      INTEGER NOT NULL DEFAULT 0,
    response_code INTEGER,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_webhook_deliveries_endpoint ON webhook_deliveries(endpoint_id);

-- ---------------------------------------------------------------------------
-- ingest_cursor: durable Horizon paging token per (wallet, network) for resume/replay.
-- ---------------------------------------------------------------------------
CREATE TABLE ingest_cursor (
    wallet_id   UUID PRIMARY KEY REFERENCES wallets(id) ON DELETE CASCADE,
    paging_token TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

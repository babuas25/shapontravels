CREATE TABLE administrators (
    id UUID PRIMARY KEY,
    username TEXT NOT NULL UNIQUE CHECK (length(username) BETWEEN 3 AND 100),
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'super_admin')),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE bootstrap_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    administrator_id UUID NOT NULL REFERENCES administrators(id),
    completed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE admin_sessions (
    token_hash BYTEA PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    administrator_id UUID NOT NULL REFERENCES administrators(id),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX admin_sessions_expiry_idx ON admin_sessions(expires_at);
CREATE TABLE api_clients (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    audience TEXT NOT NULL CHECK (audience IN ('b2b', 'b2c')),
    agent_id UUID,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    permissions TEXT[] NOT NULL DEFAULT ARRAY['search:read']::TEXT[]
        CHECK (permissions <@ ARRAY['search:read', 'booking', 'cancellation', 'ticketing']::TEXT[]),
    rate_limit_per_minute INTEGER NOT NULL DEFAULT 60 CHECK (rate_limit_per_minute BETWEEN 1 AND 10000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (audience = 'b2b' OR agent_id IS NULL)
);
CREATE TABLE client_credentials (
    id UUID PRIMARY KEY,
    client_id UUID NOT NULL REFERENCES api_clients(id),
    secret_hash TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX client_one_active_credential_idx ON client_credentials(client_id) WHERE active;
CREATE TABLE machine_tokens (
    token_hash BYTEA PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    client_id UUID NOT NULL REFERENCES api_clients(id),
    credential_id UUID NOT NULL REFERENCES client_credentials(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '1800 seconds',
    CHECK (expires_at = created_at + INTERVAL '1800 seconds')
);
CREATE INDEX machine_tokens_expiry_idx ON machine_tokens(expires_at);
-- Shared across API instances; keys are opaque hashes, never raw passwords or tokens.
CREATE TABLE rate_buckets (
    bucket_key BYTEA PRIMARY KEY,
    window_start TIMESTAMPTZ NOT NULL DEFAULT now(),
    requests INTEGER NOT NULL DEFAULT 1
);
-- References will be populated by the search/booking steps. Ownership is checked server-side.
CREATE TABLE owned_resources (
    id UUID PRIMARY KEY,
    client_id UUID NOT NULL REFERENCES api_clients(id),
    kind TEXT NOT NULL CHECK (kind IN ('search', 'offer', 'price', 'booking', 'ticket')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, client_id)
);
CREATE INDEX owned_resources_client_idx ON owned_resources(client_id, kind);

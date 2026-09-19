-- Search usage is independent of short-lived offer retention.
CREATE TABLE search_supplier_limits (
    supplier text PRIMARY KEY REFERENCES supplier_connections(id),
    daily_limit integer CHECK (daily_limit BETWEEN 0 AND 1000000),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0)
);
INSERT INTO search_supplier_limits(supplier) SELECT id FROM supplier_connections;
CREATE TABLE search_user_controls (
    subject text PRIMARY KEY REFERENCES portal_users(clerk_user_id),
    search_enabled boolean NOT NULL DEFAULT true,
    daily_limit integer CHECK (daily_limit BETWEEN 1 AND 1000000),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0)
);
CREATE TABLE search_daily_hits (
    day date NOT NULL,
    kind text NOT NULL CHECK (kind IN ('actor','supplier')),
    key text NOT NULL,
    hits bigint NOT NULL DEFAULT 0 CHECK (hits >= 0),
    PRIMARY KEY(day,kind,key)
);
CREATE TABLE search_usage (
    id uuid PRIMARY KEY,
    client_id uuid NOT NULL REFERENCES api_clients(id),
    subject text,
    actor_key text NOT NULL,
    routes jsonb NOT NULL,
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    outcome text NOT NULL DEFAULT 'pending' CHECK (outcome IN ('pending','success','failed','blocked')),
    total_ms bigint,
    error_code text
);
CREATE INDEX search_usage_period ON search_usage(started_at);
CREATE INDEX search_usage_subject_period ON search_usage(subject,started_at);
CREATE TABLE search_supplier_usage (
    search_id uuid NOT NULL REFERENCES search_usage(id),
    supplier text NOT NULL REFERENCES supplier_connections(id),
    dispatched boolean NOT NULL,
    outcome text NOT NULL CHECK (outcome IN ('pending','success','failed','blocked')),
    PRIMARY KEY(search_id,supplier)
);

CREATE TABLE flight_searches (
    id UUID PRIMARY KEY,
    client_id UUID NOT NULL REFERENCES api_clients(id),
    request JSONB NOT NULL,
    currency TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    UNIQUE(id,client_id)
);
CREATE TABLE flight_offers (
    id UUID PRIMARY KEY,
    client_id UUID NOT NULL REFERENCES api_clients(id),
    search_id UUID NOT NULL,
    supplier_id TEXT NOT NULL REFERENCES supplier_connections(id),
    availability_epoch BIGINT NOT NULL,
    original JSONB NOT NULL,
    selling JSONB NOT NULL,
    -- Maps original opaque strings to generated client-facing UUIDs.
    reference_map JSONB NOT NULL,
    rule_id UUID NOT NULL,
    rule_version BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY(search_id,client_id) REFERENCES flight_searches(id,client_id),
    FOREIGN KEY(rule_id,rule_version) REFERENCES markup_rule_versions(rule_id,version),
    UNIQUE(id,client_id)
);
CREATE INDEX flight_offers_expiry_idx ON flight_offers(expires_at);
CREATE INDEX flight_searches_expiry_idx ON flight_searches(expires_at);

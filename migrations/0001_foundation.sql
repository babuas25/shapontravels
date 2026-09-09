-- Credentials belong in the secret store/environment, never in this table.
CREATE TABLE supplier_connections (
    id TEXT PRIMARY KEY CHECK (id IN ('firsttrip', 'takeoff', 'triplover')),
    search_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    servicing_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    booking_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ticketing_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    timeout_seconds INTEGER NOT NULL DEFAULT 20 CHECK (timeout_seconds BETWEEN 1 AND 120),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO supplier_connections (id) VALUES ('firsttrip'), ('takeoff'), ('triplover');

CREATE TABLE audit_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('system', 'admin', 'client')),
    actor_id TEXT,
    action TEXT NOT NULL,
    resource_kind TEXT NOT NULL,
    resource_id TEXT,
    correlation_id UUID,
    -- Only allowlisted non-sensitive operational metadata may be stored.
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object')
);
CREATE INDEX audit_events_resource_idx ON audit_events (resource_kind, resource_id, occurred_at);

CREATE FUNCTION reject_audit_mutation() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit events are append-only';
END;
$$;
CREATE TRIGGER audit_events_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON audit_events
FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

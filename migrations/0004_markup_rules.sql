-- Draft persistence is independent of the still-pending active-duplicate policy.
CREATE TABLE markup_rules (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    audience TEXT NOT NULL CHECK (audience IN ('b2b','b2c','specific_agent')),
    agent_id UUID,
    airline TEXT CHECK (airline ~ '^[A-Z0-9]{2}$'),
    origin TEXT CHECK (origin ~ '^[A-Z]{3}$'),
    destination TEXT CHECK (destination ~ '^[A-Z]{3}$'),
    kind TEXT NOT NULL CHECK (kind IN ('fixed','percentage')),
    amount NUMERIC NOT NULL CHECK (amount >= 0 AND amount < 'Infinity'::numeric),
    currency TEXT NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    active BOOLEAN NOT NULL DEFAULT FALSE,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((audience='specific_agent') = (agent_id IS NOT NULL)),
    CHECK ((origin IS NULL) = (destination IS NULL)),
    CHECK (origin IS NULL OR origin <> destination)
);
CREATE INDEX markup_rules_audience_idx ON markup_rules(audience,agent_id) WHERE active;
-- Prior versions retain the original rule definition for future pricing snapshots.
CREATE TABLE markup_rule_versions (
    rule_id UUID NOT NULL REFERENCES markup_rules(id),
    version BIGINT NOT NULL,
    definition JSONB NOT NULL,
    changed_by UUID NOT NULL REFERENCES administrators(id),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(rule_id,version)
);
CREATE TRIGGER markup_rule_versions_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON markup_rule_versions
FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

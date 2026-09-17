-- Phase 2 is deliberately staged. There is no canonical/live mode yet.
CREATE TABLE portal_identity_control (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    authority_mode TEXT NOT NULL DEFAULT 'staged' CHECK (authority_mode = 'staged'),
    bootstrap_user_id UUID NOT NULL REFERENCES portal_users(id),
    bootstrap_operator TEXT NOT NULL CHECK (octet_length(bootstrap_operator) BETWEEN 1 AND 128),
    bootstrapped_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER portal_identity_control_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_control
    FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

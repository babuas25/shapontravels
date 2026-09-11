-- Append-only revalidation of a captured successful ticket response after a
-- validator compatibility fix. The original dispatch/outcome is never rewritten.
CREATE TABLE flight_ticket_verifications (
 issue_id UUID PRIMARY KEY REFERENCES flight_ticket_issues(id),
 public_response JSONB NOT NULL,
 client_id UUID NOT NULL REFERENCES api_clients(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER ticket_verification_immutable BEFORE UPDATE OR DELETE OR TRUNCATE
 ON flight_ticket_verifications FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

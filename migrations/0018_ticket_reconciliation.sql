CREATE TABLE flight_ticket_reconciliations (
 id UUID PRIMARY KEY,
 issue_id UUID NOT NULL REFERENCES flight_ticket_issues(id),
 client_id UUID NOT NULL REFERENCES api_clients(id),
 actor_kind TEXT NOT NULL CHECK(actor_kind IN ('client','admin')),
 actor_id UUID NOT NULL,
 pnr_response JSONB,
 report_response JSONB,
 public_response JSONB,
 result TEXT NOT NULL CHECK(result IN ('verified','insufficient','already_resolved')),
 error_code TEXT,
 requested_at TIMESTAMPTZ NOT NULL,
 completed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK ((result='verified') = (public_response IS NOT NULL))
);
CREATE INDEX ticket_reconciliation_issue_idx ON flight_ticket_reconciliations(issue_id,requested_at DESC);
CREATE TRIGGER ticket_reconciliation_immutable BEFORE UPDATE OR DELETE OR TRUNCATE
 ON flight_ticket_reconciliations FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

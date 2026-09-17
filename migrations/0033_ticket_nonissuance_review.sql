-- Manual supplier confirmation is an attestation, never an inferred negative
-- from PNR status, an empty report, a timeout, or an expired deadline.
CREATE TABLE ticket_nonissuance_proposals (
 id UUID PRIMARY KEY,
 issue_id UUID NOT NULL REFERENCES flight_ticket_issues(id),
 operation_id UUID NOT NULL REFERENCES wallet_operations(id),
 requested_by TEXT NOT NULL,
 requested_role TEXT NOT NULL CHECK(requested_role IN ('superadmin','admin','staff_account')),
 evidence JSONB NOT NULL CHECK(jsonb_typeof(evidence)='object'),
 snapshot_hash TEXT NOT NULL CHECK(snapshot_hash ~ '^[a-f0-9]{64}$'),
 request_hash BYTEA NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX ticket_nonissuance_issue_idx ON ticket_nonissuance_proposals(issue_id,created_at DESC);
CREATE TABLE ticket_nonissuance_decisions (
 proposal_id UUID PRIMARY KEY REFERENCES ticket_nonissuance_proposals(id),
 issue_id UUID NOT NULL REFERENCES flight_ticket_issues(id),
 decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
 reviewed_by TEXT NOT NULL,
 reviewed_role TEXT NOT NULL CHECK(reviewed_role IN ('superadmin','admin','staff_account')),
 remarks TEXT NOT NULL CHECK(length(btrim(remarks)) BETWEEN 3 AND 1000),
 request_hash BYTEA NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX ticket_nonissuance_release_once ON ticket_nonissuance_decisions(issue_id) WHERE decision='approved';
CREATE TRIGGER nonissuance_proposals_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON ticket_nonissuance_proposals FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER nonissuance_decisions_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON ticket_nonissuance_decisions FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

CREATE FUNCTION ticket_positive_signal(body JSONB) RETURNS BOOLEAN LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE k TEXT; v JSONB;
BEGIN
 IF jsonb_typeof(body)='object' THEN
   FOR k,v IN SELECT key,value FROM jsonb_each(body) LOOP
     IF ((lower(k) LIKE '%ticketnumber%' OR lower(k)='ticketcoderef') AND v NOT IN ('null'::jsonb,'""'::jsonb,'[]'::jsonb))
        OR (lower(k) IN ('status','bookingstatus') AND lower(v#>>'{}') IN ('ticketed','issued'))
        OR ticket_positive_signal(v) THEN RETURN TRUE; END IF;
   END LOOP;
 ELSIF jsonb_typeof(body)='array' THEN
   FOR v IN SELECT value FROM jsonb_array_elements(body) LOOP
     IF ticket_positive_signal(v) THEN RETURN TRUE; END IF;
   END LOOP;
 END IF;
 RETURN FALSE;
END;
$$;

CREATE VIEW ticket_nonissuance_positive_observations AS
 SELECT booking_id FROM flight_booking_pnr_observations WHERE ticket_positive_signal(response)
 UNION SELECT booking_id FROM flight_ticket_reports WHERE verified OR ticket_positive_signal(original_response)
 UNION SELECT t.booking_id FROM flight_ticket_reconciliations r JOIN flight_ticket_issues t ON t.id=r.issue_id
   WHERE r.result='verified' OR ticket_positive_signal(r.pnr_response) OR ticket_positive_signal(r.report_response);

CREATE FUNCTION ticket_nonissuance_integrity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p ticket_nonissuance_proposals; d ticket_nonissuance_decisions; t flight_ticket_issues; w wallet_operations;
BEGIN
 IF TG_TABLE_NAME='wallet_ledger_entries' THEN
   IF NEW.transaction_type<>'hold_release' THEN RETURN NULL; END IF;
   SELECT * INTO w FROM wallet_operations WHERE id=NEW.operation_id;
   IF w.subject_kind<>'ticket_issue' THEN RETURN NULL; END IF;
   SELECT * INTO d FROM ticket_nonissuance_decisions WHERE issue_id=w.subject_id AND decision='approved';
   IF NOT FOUND THEN RAISE EXCEPTION 'NONISSUANCE_APPROVAL_REQUIRED'; END IF;
 ELSE
   d := NEW;
 END IF;
 SELECT * INTO p FROM ticket_nonissuance_proposals WHERE id=d.proposal_id;
 IF p.issue_id<>d.issue_id OR p.requested_by=d.reviewed_by THEN
   RAISE EXCEPTION 'NONISSUANCE_INDEPENDENT_REVIEW_REQUIRED';
 END IF;
 IF d.decision='rejected' THEN RETURN NULL; END IF;
 SELECT * INTO t FROM flight_ticket_issues WHERE id=p.issue_id;
 SELECT * INTO w FROM wallet_operations WHERE id=p.operation_id;
 IF t.state<>'outcome_unknown' OR NOT t.wallet_required
    OR EXISTS(SELECT 1 FROM flight_ticket_verifications WHERE issue_id=t.id)
    OR ticket_positive_signal(t.original_response)
    OR t.original_response#>'{item2,isSuccess}'='true'::jsonb
    OR EXISTS(SELECT 1 FROM ticket_nonissuance_positive_observations WHERE booking_id=t.booking_id)
    OR w.subject_kind<>'ticket_issue' OR w.subject_id<>t.id
    OR w.booking_id IS DISTINCT FROM t.booking_id OR w.state<>'released'
    OR NOT EXISTS(SELECT 1 FROM wallet_ledger_entries l WHERE l.operation_id=w.id AND l.transaction_type='hold_release' AND l.amount=w.amount AND l.created_by_user_id=d.reviewed_by) THEN
   RAISE EXCEPTION 'NONISSUANCE_RELEASE_MISMATCH';
 END IF;
 RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER ticket_nonissuance_decision_integrity AFTER INSERT ON ticket_nonissuance_decisions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ticket_nonissuance_integrity();
CREATE CONSTRAINT TRIGGER ticket_nonissuance_ledger_integrity AFTER INSERT ON wallet_ledger_entries DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ticket_nonissuance_integrity();

-- Positive evidence always wins presentation, even if a manual confirmation
-- was wrong. Such a contradiction remains visible for financial recovery.
CREATE VIEW flight_ticket_outcomes AS
 SELECT t.id, CASE WHEN t.state='issued' OR v.issue_id IS NOT NULL THEN 'issued'
   WHEN d.issue_id IS NOT NULL AND w.state='released'
     AND NOT EXISTS(SELECT 1 FROM ticket_nonissuance_positive_observations p WHERE p.booking_id=t.booking_id) THEN 'not_issued'
   ELSE t.state END AS state,
   COALESCE(v.created_at,d.created_at,t.updated_at) AS updated_at
 FROM flight_ticket_issues t
 LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id
 LEFT JOIN ticket_nonissuance_decisions d ON d.issue_id=t.id AND d.decision='approved'
 LEFT JOIN wallet_operations w ON w.subject_kind='ticket_issue' AND w.subject_id=t.id;

ALTER TABLE flight_bookings ADD COLUMN last_reconciliation_verified BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE flight_bookings DROP CONSTRAINT flight_bookings_state_check;
ALTER TABLE flight_bookings ADD CONSTRAINT flight_bookings_state_check CHECK(state IN ('pending','held','outcome_unknown','manually_resolved'));
CREATE TABLE booking_resolutions (
 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 booking_id UUID NOT NULL REFERENCES flight_bookings(id),
 administrator_id UUID NOT NULL REFERENCES administrators(id),
 outcome TEXT NOT NULL CHECK(outcome IN ('held','not_created','issued','cancelled')),
 reason TEXT NOT NULL CHECK(length(reason) BETWEEN 10 AND 2000),
 evidence TEXT NOT NULL CHECK(length(evidence) BETWEEN 10 AND 4000),
 supplier_case_ref TEXT NOT NULL CHECK(length(supplier_case_ref) BETWEEN 3 AND 200),
 previous_state TEXT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER booking_resolutions_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON booking_resolutions
FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE INDEX flight_bookings_unresolved_idx ON flight_bookings(created_at,id) WHERE state IN ('pending','outcome_unknown');

CREATE INDEX booking_resolutions_history_idx ON booking_resolutions(booking_id,id DESC);
CREATE TABLE booking_late_outcomes (
 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 booking_id UUID NOT NULL REFERENCES flight_bookings(id),
 response JSONB,
 received_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER booking_late_outcomes_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON booking_late_outcomes
FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

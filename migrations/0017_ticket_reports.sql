CREATE TABLE flight_ticket_reports (
 id UUID PRIMARY KEY,
 booking_id UUID NOT NULL,
 client_id UUID NOT NULL,
 original_response JSONB NOT NULL,
 public_response JSONB,
 verified BOOLEAN NOT NULL,
 requested_at TIMESTAMPTZ NOT NULL,
 received_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(booking_id,client_id) REFERENCES flight_bookings(id,client_id),
 CHECK (verified = (public_response IS NOT NULL))
);
CREATE INDEX flight_ticket_reports_booking_idx ON flight_ticket_reports(booking_id,requested_at DESC);
CREATE TRIGGER ticket_reports_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON flight_ticket_reports
 FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

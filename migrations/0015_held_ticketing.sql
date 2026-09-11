-- A booking has one durable issue reservation, independent of the original Hold reply.
ALTER TABLE flight_bookings ADD CONSTRAINT flight_bookings_ticket_owner UNIQUE(id,client_id);
CREATE TABLE flight_ticket_issues (
 id UUID PRIMARY KEY,
 booking_id UUID NOT NULL UNIQUE,
 client_id UUID NOT NULL,
 idempotency_key TEXT NOT NULL CHECK(length(idempotency_key) BETWEEN 1 AND 128),
 request_hash BYTEA NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','issued','outcome_unknown')),
 request JSONB NOT NULL,
 preflight JSONB NOT NULL,
 original_response JSONB,
 public_response JSONB,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(booking_id,client_id) REFERENCES flight_bookings(id,client_id),
 UNIQUE(client_id,idempotency_key)
);
CREATE FUNCTION protect_ticket_issue() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP <> 'UPDATE' THEN RAISE EXCEPTION 'ticket issue evidence cannot be removed'; END IF;
 IF OLD.state <> 'pending' OR NEW.state = 'pending'
    OR (NEW.id,NEW.booking_id,NEW.client_id,NEW.idempotency_key,NEW.request_hash,NEW.request,NEW.preflight,NEW.created_at)
       IS DISTINCT FROM (OLD.id,OLD.booking_id,OLD.client_id,OLD.idempotency_key,OLD.request_hash,OLD.request,OLD.preflight,OLD.created_at)
 THEN RAISE EXCEPTION 'ticket issue reservation/evidence is immutable'; END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER ticket_issue_immutable BEFORE UPDATE OR DELETE ON flight_ticket_issues
 FOR EACH ROW EXECUTE FUNCTION protect_ticket_issue();
CREATE TRIGGER ticket_issue_no_truncate BEFORE TRUNCATE ON flight_ticket_issues
 FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

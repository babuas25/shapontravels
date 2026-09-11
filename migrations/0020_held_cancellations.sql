CREATE TABLE flight_cancellations (
 id UUID PRIMARY KEY,
 booking_id UUID NOT NULL UNIQUE,
 client_id UUID NOT NULL,
 idempotency_key TEXT NOT NULL CHECK(length(idempotency_key) BETWEEN 1 AND 128),
 request_hash BYTEA NOT NULL,
 request JSONB NOT NULL,
 preflight JSONB NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','cancelled','outcome_unknown')),
 original_response JSONB,
 public_response JSONB,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 FOREIGN KEY(booking_id,client_id) REFERENCES flight_bookings(id,client_id),
 UNIQUE(client_id,idempotency_key)
);
CREATE FUNCTION protect_cancellation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP <> 'UPDATE' THEN RAISE EXCEPTION 'cancellation evidence cannot be removed'; END IF;
 IF OLD.state <> 'pending' OR NEW.state='pending'
 OR (NEW.id,NEW.booking_id,NEW.client_id,NEW.idempotency_key,NEW.request_hash,NEW.request,NEW.preflight,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.booking_id,OLD.client_id,OLD.idempotency_key,OLD.request_hash,OLD.request,OLD.preflight,OLD.created_at)
 THEN RAISE EXCEPTION 'cancellation reservation/evidence is immutable'; END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER cancellation_immutable BEFORE UPDATE OR DELETE ON flight_cancellations
 FOR EACH ROW EXECUTE FUNCTION protect_cancellation();
CREATE TRIGGER cancellation_no_truncate BEFORE TRUNCATE ON flight_cancellations
 FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
-- Serialize both mutation reservations even for writers outside the HTTP handlers.
CREATE FUNCTION exclude_issue_and_cancel() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM id FROM flight_bookings WHERE id=NEW.booking_id FOR UPDATE;
 IF TG_TABLE_NAME='flight_cancellations' THEN
  IF EXISTS(SELECT 1 FROM flight_ticket_issues WHERE booking_id=NEW.booking_id) THEN
   RAISE EXCEPTION 'issue already reserved';
  END IF;
 ELSE
  IF EXISTS(SELECT 1 FROM flight_cancellations WHERE booking_id=NEW.booking_id) THEN
   RAISE EXCEPTION 'cancel already reserved';
  END IF;
 END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER cancellation_excludes_issue BEFORE INSERT ON flight_cancellations
 FOR EACH ROW EXECUTE FUNCTION exclude_issue_and_cancel();
CREATE TRIGGER issue_excludes_cancellation BEFORE INSERT ON flight_ticket_issues
 FOR EACH ROW EXECUTE FUNCTION exclude_issue_and_cancel();
CREATE TABLE flight_cancellation_reconciliations (
 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 cancellation_id UUID NOT NULL REFERENCES flight_cancellations(id),
 response JSONB,
 verified_cancelled BOOLEAN NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER cancellation_reconciliations_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON flight_cancellation_reconciliations
 FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

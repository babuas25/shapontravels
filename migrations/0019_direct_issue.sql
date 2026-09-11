-- Direct dispatch uses the existing durable booking reservation.
ALTER TABLE flight_bookings ADD COLUMN execution_mode TEXT NOT NULL DEFAULT 'hold'
 CHECK(execution_mode IN ('hold','direct'));
ALTER TABLE flight_bookings DROP CONSTRAINT flight_bookings_state_check;
ALTER TABLE flight_bookings ADD CONSTRAINT flight_bookings_state_check
 CHECK(state IN ('pending','held','issued','outcome_unknown','manually_resolved'));
CREATE FUNCTION protect_booking_execution_mode() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.execution_mode IS DISTINCT FROM OLD.execution_mode THEN
  RAISE EXCEPTION 'booking execution mode is immutable';
 END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER booking_execution_mode_immutable BEFORE UPDATE ON flight_bookings
 FOR EACH ROW EXECUTE FUNCTION protect_booking_execution_mode();
-- Preserve intentional repeat Holds, but never repeat an instant purchase.
CREATE UNIQUE INDEX flight_bookings_direct_offer_unique ON flight_bookings(offer_id)
 WHERE execution_mode='direct';

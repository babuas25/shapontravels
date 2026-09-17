-- Keep verified evidence even when a later lookup fails. The existing columns
-- remain the latest-attempt view used by status/Admin/Cancel; ticket issue reads
-- the latest verified observation without making a supplier request.
CREATE TABLE flight_booking_pnr_observations (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    booking_id UUID NOT NULL REFERENCES flight_bookings(id),
    response JSONB NOT NULL,
    verified BOOLEAN NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX booking_pnr_verified_idx
    ON flight_booking_pnr_observations(booking_id, requested_at DESC, id DESC)
    WHERE verified;

-- Preserve the evidence still available at upgrade time. Older responses that
-- were already overwritten cannot be reconstructed.
INSERT INTO flight_booking_pnr_observations(booking_id,response,verified,requested_at)
SELECT id,last_reconciliation,last_reconciliation_verified,COALESCE(reconciled_at,updated_at)
FROM flight_bookings WHERE last_reconciliation IS NOT NULL;

CREATE TRIGGER booking_pnr_observations_immutable
    BEFORE UPDATE OR DELETE OR TRUNCATE ON flight_booking_pnr_observations
    FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

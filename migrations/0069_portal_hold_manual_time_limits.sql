-- Staff-set operational cutoffs are separate from immutable supplier PNR and
-- Book evidence. A later decision supersedes an earlier one without erasing it.
CREATE TABLE portal_hold_manual_time_limits (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    booking_id UUID NOT NULL REFERENCES flight_bookings(id),
    deadline_at TIMESTAMPTZ NOT NULL,
    actor_subject TEXT NOT NULL,
    actor_role TEXT NOT NULL CHECK (actor_role IN ('superadmin', 'admin', 'staff_support')),
    reason TEXT NOT NULL CHECK (char_length(reason) BETWEEN 10 AND 500),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX portal_hold_manual_time_limits_latest_idx
    ON portal_hold_manual_time_limits(booking_id, id DESC);
CREATE TRIGGER portal_hold_manual_time_limits_immutable
    BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_hold_manual_time_limits
    FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

-- Match ticketing's two trusted deadline formats. Original Book evidence only
-- accepts an explicit offset; verified PNR also accepts Bangladesh local time.
CREATE FUNCTION portal_hold_supplier_deadline_known(raw TEXT, verified_pnr BOOLEAN)
RETURNS BOOLEAN LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF raw IS NULL THEN
        RETURN FALSE;
    END IF;
    IF raw ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})$' THEN
        PERFORM raw::timestamptz;
        RETURN TRUE;
    END IF;
    IF verified_pnr AND raw ~ '^[0-9]{1,2} [A-Za-z]{3} [0-9]{4}, [0-9]{1,2}:[0-9]{2} (AM|PM)$' THEN
        PERFORM to_timestamp(raw, 'DD Mon YYYY, HH12:MI AM');
        RETURN TRUE;
    END IF;
    RETURN FALSE;
EXCEPTION WHEN invalid_datetime_format OR datetime_field_overflow THEN
    RETURN FALSE;
END;
$$;

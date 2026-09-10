-- Supplier-derived references are nullable until both locators are known.
ALTER TABLE flight_bookings ADD COLUMN public_ref TEXT
    CHECK (public_ref ~ '^STR[A-Z0-9]{12}$');
CREATE INDEX booking_public_ref_lookup ON flight_bookings(client_id, public_ref)
    WHERE public_ref IS NOT NULL;

CREATE FUNCTION booking_reference_from_evidence(book JSONB, lookup JSONB, verified BOOLEAN)
RETURNS TEXT LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    locator TEXT;
    airline_values JSONB;
    airline TEXT;
    distinct_count INTEGER;
BEGIN
    IF book #>> '{item2,isSuccess}' IS DISTINCT FROM 'true' THEN RETURN NULL; END IF;
    locator := book #>> '{item1,pnr}';
    IF locator IS NULL OR locator !~ '^[A-Z0-9]{6}$' THEN RETURN NULL; END IF;
    airline_values := book #> '{item1,airlinesPNR}';
    IF airline_values IS NULL OR airline_values = 'null'::jsonb OR airline_values = '[]'::jsonb THEN
        airline_values := book #> '{item1,airlinePNRs}';
    END IF;
    IF (airline_values IS NULL OR airline_values = 'null'::jsonb OR airline_values = '[]'::jsonb)
        AND verified AND lookup #>> '{item1,pnr}' = locator THEN
        airline_values := lookup #> '{item1,airlinePNRs}';
    END IF;
    IF jsonb_typeof(airline_values) IS DISTINCT FROM 'array' THEN RETURN NULL; END IF;
    IF jsonb_array_length(airline_values) = 0 THEN RETURN NULL; END IF;
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(airline_values) v
        WHERE jsonb_typeof(v) <> 'string' OR (v #>> '{}') !~ '^[A-Z0-9]{6}$') THEN RETURN NULL; END IF;
    SELECT count(DISTINCT v), min(v) INTO distinct_count, airline FROM jsonb_array_elements_text(airline_values) v;
    IF distinct_count <> 1 THEN RETURN NULL; END IF;
    RETURN 'STR' || locator || airline;
END;
$$;

UPDATE flight_bookings SET public_ref = booking_reference_from_evidence(
    original_response, last_reconciliation, last_reconciliation_verified);

CREATE FUNCTION protect_booking_public_reference() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE derived TEXT;
BEGIN
    IF TG_OP = 'UPDATE' AND OLD.public_ref IS NOT NULL THEN
        IF NEW.public_ref IS DISTINCT FROM OLD.public_ref THEN
            RAISE EXCEPTION 'booking public reference is immutable';
        END IF;
        RETURN NEW;
    END IF;
    derived := booking_reference_from_evidence(NEW.original_response, NEW.last_reconciliation, NEW.last_reconciliation_verified);
    IF NEW.public_ref IS NOT NULL AND NEW.public_ref IS DISTINCT FROM derived THEN
        RAISE EXCEPTION 'booking public reference must match supplier evidence';
    END IF;
    NEW.public_ref := derived;
    RETURN NEW;
END;
$$;
CREATE TRIGGER booking_public_reference_immutable BEFORE INSERT OR UPDATE
    ON flight_bookings FOR EACH ROW EXECUTE FUNCTION protect_booking_public_reference();

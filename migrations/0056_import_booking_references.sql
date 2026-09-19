-- Preserve old import URLs and immutable financial records; use the same
-- evidence-derived public reference as native bookings on every read surface.
CREATE FUNCTION import_booking_reference(data JSONB) RETURNS TEXT LANGUAGE SQL IMMUTABLE AS $$
 SELECT booking_reference_from_evidence(jsonb_build_object('item2',jsonb_build_object('isSuccess',true),'item1',jsonb_build_object('pnr',data->'pnr','airlinesPNR',data->'airlinesPnr')),NULL,false)
$$;
ALTER TABLE portal_import_bookings ADD COLUMN booking_reference TEXT GENERATED ALWAYS AS (import_booking_reference(data)) STORED;
ALTER TABLE portal_import_bookings ADD COLUMN display_supplier_reference TEXT GENERATED ALWAYS AS (
 CASE WHEN source='IMP_EXP' THEN nullif(upper(trim(data->>'originalReference')),'') ELSE supplier_reference END
) STORED;
CREATE UNIQUE INDEX portal_import_booking_reference ON portal_import_bookings(booking_reference) WHERE booking_reference IS NOT NULL;

-- Audited corrections to invoice cost never rewrite captured wallet amounts.
CREATE TABLE portal_import_cost_corrections (
 booking_id UUID PRIMARY KEY REFERENCES portal_import_bookings(id),
 supplier_minor BIGINT NOT NULL CHECK(supplier_minor>0 AND supplier_minor<=99999999999999),
 reason TEXT NOT NULL CHECK(length(reason) BETWEEN 10 AND 500),
 actor_id UUID NOT NULL REFERENCES portal_users(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER import_cost_correction_immutable BEFORE UPDATE OR DELETE ON portal_import_cost_corrections FOR EACH ROW EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER import_cost_correction_no_truncate BEFORE TRUNCATE ON portal_import_cost_corrections FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

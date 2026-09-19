-- Stable platform references for imports exposed through the standard read APIs.
CREATE TABLE portal_import_api_references (
 booking_id UUID PRIMARY KEY REFERENCES portal_import_bookings(id),
 transaction_id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
 item_id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
 price_id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
 ticket_id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid()
);
INSERT INTO portal_import_api_references(booking_id) SELECT id FROM portal_import_bookings;
CREATE FUNCTION create_import_api_references() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 INSERT INTO portal_import_api_references(booking_id) VALUES(NEW.id);
 RETURN NEW;
END $$;
CREATE TRIGGER import_api_references_created AFTER INSERT ON portal_import_bookings FOR EACH ROW EXECUTE FUNCTION create_import_api_references();
CREATE TRIGGER import_api_references_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_import_api_references FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

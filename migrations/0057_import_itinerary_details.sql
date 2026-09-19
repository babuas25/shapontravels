-- Supplement source-provided display details without rewriting issued bookings.
CREATE TABLE portal_import_itinerary_details (
 id UUID PRIMARY KEY,
 booking_id UUID NOT NULL REFERENCES portal_import_bookings(id),
 itinerary JSONB NOT NULL CHECK(jsonb_typeof(itinerary)='object'),
 actor_id UUID NOT NULL REFERENCES portal_users(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX import_itinerary_latest ON portal_import_itinerary_details(booking_id,created_at DESC,id DESC);
CREATE TRIGGER import_itinerary_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_import_itinerary_details FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

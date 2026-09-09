-- A new client-scoped idempotency key represents a new user booking intent.
-- Retain UNIQUE(client_id,idempotency_key) for safe retries of the same intent.
ALTER TABLE flight_bookings DROP CONSTRAINT flight_bookings_offer_id_key;
CREATE INDEX flight_bookings_offer_idx ON flight_bookings(offer_id);

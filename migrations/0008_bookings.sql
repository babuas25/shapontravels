ALTER TABLE flight_reprices ADD CONSTRAINT flight_reprices_booking_owner UNIQUE(id,offer_id,client_id);
CREATE TABLE flight_bookings (
 id UUID PRIMARY KEY,
 client_id UUID NOT NULL REFERENCES api_clients(id),
 offer_id UUID NOT NULL,
 price_id UUID NOT NULL,
 supplier_id TEXT NOT NULL REFERENCES supplier_connections(id),
 idempotency_key TEXT NOT NULL CHECK(length(idempotency_key) BETWEEN 1 AND 128),
 request_hash BYTEA NOT NULL,
 request JSONB NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','held','outcome_unknown')),
 original_response JSONB,
 public_response JSONB,
 pnr TEXT,
 supplier_booking_ref TEXT,
 ticketing_time_limit TEXT,
 error_code TEXT,
 last_reconciliation JSONB,
 reconciled_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 FOREIGN KEY(price_id,offer_id,client_id) REFERENCES flight_reprices(id,offer_id,client_id),
 FOREIGN KEY(offer_id,client_id) REFERENCES flight_offers(id,client_id),
 UNIQUE(client_id,idempotency_key),
 -- Even a different idempotency key or pricing version must not repeat a mutation.
 UNIQUE(offer_id)
);
CREATE INDEX flight_bookings_client_idx ON flight_bookings(client_id,created_at DESC);

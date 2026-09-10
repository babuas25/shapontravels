-- Payloads are removed after 15 minutes only when expired and unreferenced.
-- Small owner-scoped markers preserve 410 responses for 24 hours after cleanup.
CREATE TABLE expired_flight_offers (
    id UUID PRIMARY KEY,
    client_id UUID NOT NULL REFERENCES api_clients(id),
    purged_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX expired_flight_offers_purged_idx ON expired_flight_offers(purged_at);
CREATE INDEX flight_offers_cleanup_idx ON flight_offers(created_at,id);
CREATE INDEX flight_offers_search_idx ON flight_offers(search_id);
CREATE INDEX flight_searches_cleanup_idx ON flight_searches(created_at,id);

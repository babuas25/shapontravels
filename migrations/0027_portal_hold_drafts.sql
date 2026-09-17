-- Owner and creator are distinct. A draft's UUID is also its stable Book key.
CREATE TABLE portal_hold_drafts (
    id UUID PRIMARY KEY,
    creator_external_user_id TEXT NOT NULL,
    owner_external_user_id TEXT NOT NULL,
    client_id UUID NOT NULL REFERENCES api_clients(id),
    source_offer_id UUID NOT NULL REFERENCES flight_offers(id),
    offer_id UUID NOT NULL REFERENCES flight_offers(id),
    price_id UUID REFERENCES flight_reprices(id),
    selection JSONB NOT NULL,
    owner_display JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(offer_id)
);
CREATE INDEX portal_hold_drafts_owner_idx ON portal_hold_drafts(client_id,created_at DESC);
ALTER TABLE flight_bookings ADD COLUMN portal_hold_draft_id UUID UNIQUE REFERENCES portal_hold_drafts(id);
ALTER TABLE flight_bookings ADD COLUMN created_by_external_user_id TEXT;
ALTER TABLE flight_bookings ADD COLUMN portal_customer_contact JSONB;
ALTER TABLE flight_bookings ADD CONSTRAINT portal_hold_creator CHECK
    ((portal_hold_draft_id IS NULL AND created_by_external_user_id IS NULL) OR
     (portal_hold_draft_id IS NOT NULL AND created_by_external_user_id IS NOT NULL AND execution_mode='hold'));

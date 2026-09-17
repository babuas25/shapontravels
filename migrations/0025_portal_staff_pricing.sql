-- Technical ownership for staff flight review, separate from B2B memberships.
CREATE TABLE portal_staff_clients (
    external_user_id TEXT PRIMARY KEY CHECK (external_user_id LIKE 'user_%' AND length(external_user_id) <= 128),
    client_id UUID NOT NULL UNIQUE REFERENCES api_clients(id)
);
ALTER TABLE portal_prebooking_sessions ADD COLUMN staff_pricing BOOLEAN NOT NULL DEFAULT FALSE;

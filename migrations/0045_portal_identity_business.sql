-- Canonical short-lived search sessions. No legacy token or owner is adopted.
CREATE TABLE portal_identity_search_sessions (
 token_hash BYTEA PRIMARY KEY CHECK (octet_length(token_hash)=32),
 user_id UUID NOT NULL REFERENCES portal_users(id),
 authorization_version BIGINT NOT NULL,
 client_id UUID NOT NULL REFERENCES api_clients(id),
 staff_pricing BOOLEAN NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+INTERVAL '5 minutes',
 CHECK (expires_at=created_at+INTERVAL '5 minutes')
);
CREATE INDEX portal_identity_search_expiry ON portal_identity_search_sessions(expires_at);
CREATE INDEX portal_identity_search_user ON portal_identity_search_sessions(user_id);
CREATE FUNCTION identity_revoke_search_sessions() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.authorization_version<>OLD.authorization_version OR NEW.status<>OLD.status THEN
 DELETE FROM portal_identity_search_sessions WHERE user_id=NEW.id;
 END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER identity_revoke_search_sessions AFTER UPDATE ON portal_users
 FOR EACH ROW EXECUTE FUNCTION identity_revoke_search_sessions();

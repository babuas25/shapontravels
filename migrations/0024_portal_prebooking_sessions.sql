-- Server-only prebooking sessions do not grant external API or booking access.
CREATE TABLE portal_prebooking_sessions (
    token_hash BYTEA PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    client_id UUID NOT NULL REFERENCES api_clients(id),
    issuer_id UUID NOT NULL REFERENCES administrators(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '300 seconds',
    CHECK (expires_at = created_at + INTERVAL '300 seconds')
);
CREATE INDEX portal_prebooking_expiry ON portal_prebooking_sessions(expires_at);
CREATE INDEX portal_prebooking_client ON portal_prebooking_sessions(client_id);
CREATE INDEX portal_prebooking_issuer ON portal_prebooking_sessions(issuer_id);

CREATE FUNCTION revoke_portal_prebooking_sessions() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_TABLE_NAME = 'api_clients' THEN
        IF NOT NEW.active OR NOT ('search:read' = ANY(NEW.permissions)) THEN
            DELETE FROM portal_prebooking_sessions WHERE client_id = NEW.id;
        END IF;
    ELSE
        IF NOT NEW.active OR NEW.role <> 'super_admin' THEN
            DELETE FROM portal_prebooking_sessions WHERE issuer_id = NEW.id;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER revoke_client_portal_sessions AFTER UPDATE ON api_clients
FOR EACH ROW EXECUTE FUNCTION revoke_portal_prebooking_sessions();
CREATE TRIGGER revoke_issuer_portal_sessions AFTER UPDATE ON administrators
FOR EACH ROW EXECUTE FUNCTION revoke_portal_prebooking_sessions();

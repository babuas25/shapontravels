ALTER TABLE api_clients ADD COLUMN external_user_id TEXT UNIQUE;
ALTER TABLE api_clients ADD COLUMN api_management_enabled BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE api_clients ADD COLUMN management_version BIGINT NOT NULL DEFAULT 1 CHECK (management_version > 0);
ALTER TABLE api_clients ADD CONSTRAINT managed_client_b2b CHECK (external_user_id IS NULL OR audience='b2b');
-- Direct database/admin updates also invalidate tokens on loss of portal access.
CREATE FUNCTION enforce_managed_client_access() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF OLD.external_user_id IS NOT NULL AND NEW.external_user_id IS DISTINCT FROM OLD.external_user_id THEN
  RAISE EXCEPTION 'external client identity is immutable';
 END IF;
 IF NEW.external_user_id IS NOT NULL AND NEW.tier <> 'enterprise' THEN
  NEW.api_management_enabled := FALSE;
 END IF;
 IF NEW.external_user_id IS NOT NULL AND (NOT NEW.active OR NOT NEW.api_management_enabled OR NEW.tier <> 'enterprise') THEN
  DELETE FROM machine_tokens WHERE client_id=NEW.id;
 END IF;
 NEW.management_version := OLD.management_version+1;
 RETURN NEW;
END;
$$;
CREATE TRIGGER managed_client_access BEFORE UPDATE ON api_clients
FOR EACH ROW EXECUTE FUNCTION enforce_managed_client_access();

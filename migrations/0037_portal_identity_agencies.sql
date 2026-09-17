-- Explicit staged agency provisioning/reactivation. No live identity import.
ALTER TABLE portal_identity_operations DROP CONSTRAINT portal_identity_operations_action_check;
ALTER TABLE portal_identity_operations ADD CHECK(action IN ('set_role','set_access','provision_agency','reactivate_agency'));

-- Sequence values are never recycled, even on rollback. The allocator rejects
-- values above 999999 and skips all existing registry/financial owner codes.
CREATE SEQUENCE portal_identity_agency_codes AS BIGINT START WITH 100000 NO CYCLE;

CREATE TABLE portal_agency_wallets (
 agency_id UUID PRIMARY KEY REFERENCES portal_agencies(id),
 wallet_owner_id UUID NOT NULL UNIQUE REFERENCES wallet_owners(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION check_portal_agency_wallet() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM portal_agencies a JOIN wallet_owners w
   ON w.owner_type='agency' AND w.owner_key=a.agency_code
   WHERE a.id=NEW.agency_id AND w.id=NEW.wallet_owner_id) THEN
   RAISE EXCEPTION 'agency wallet identity mismatch';
 END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER portal_agency_wallet_binding BEFORE INSERT ON portal_agency_wallets
 FOR EACH ROW EXECUTE FUNCTION check_portal_agency_wallet();
CREATE TRIGGER portal_agency_wallet_retain BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_agency_wallets
 FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

-- Immutable operation result, separate from the current mutable agency version.
CREATE TABLE portal_identity_agency_results (
 operation_id UUID PRIMARY KEY REFERENCES portal_identity_operations(id),
 agency_id UUID NOT NULL REFERENCES portal_agencies(id),
 agency_version BIGINT NOT NULL CHECK(agency_version>0),
 agency_status TEXT NOT NULL CHECK(agency_status IN ('active','suspended','archived')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER portal_identity_agency_result_retain BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_agency_results
 FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

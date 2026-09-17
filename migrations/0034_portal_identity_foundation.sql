-- Additive identity foundation. No existing identities/financial rows are copied,
-- no Clerk users are changed, and this migration does not activate new authority.
CREATE TABLE portal_users (
    id UUID PRIMARY KEY,
    clerk_user_id TEXT NOT NULL UNIQUE CHECK (clerk_user_id ~ '^user_[A-Za-z0-9_]{1,123}$'),
    role TEXT NOT NULL CHECK (role IN ('superadmin','admin','staff_support','staff_account','staff_media','b2b','b2b_sub','customer')),
    status TEXT NOT NULL DEFAULT 'onboarding' CHECK (status IN ('onboarding','active','suspended','deleting','deleted')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    authorization_version BIGINT NOT NULL DEFAULT 1 CHECK (authorization_version > 0),
    email TEXT CHECK (octet_length(email) <= 320),
    first_name TEXT CHECK (octet_length(first_name) <= 400),
    last_name TEXT CHECK (octet_length(last_name) <= 400),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    deleted_at TIMESTAMPTZ,
    CHECK ((status = 'deleted') = (deleted_at IS NOT NULL)),
    UNIQUE (id, role)
);
CREATE INDEX portal_users_roster_idx ON portal_users(status, role, created_at DESC, id);

CREATE TABLE portal_agencies (
    id UUID PRIMARY KEY,
    agency_code TEXT NOT NULL UNIQUE CHECK (agency_code ~ '^ST-B2B[1-9][0-9]{5}$'),
    owner_user_id UUID NOT NULL UNIQUE,
    owner_role TEXT NOT NULL DEFAULT 'b2b' CHECK (owner_role = 'b2b'),
    owner_kind TEXT NOT NULL DEFAULT 'owner' CHECK (owner_kind = 'owner'),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','suspended','archived')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (owner_user_id, owner_role) REFERENCES portal_users(id, role)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE portal_agency_memberships (
    user_id UUID PRIMARY KEY,
    agency_id UUID NOT NULL REFERENCES portal_agencies(id),
    kind TEXT NOT NULL CHECK (kind IN ('owner','sub')),
    user_role TEXT NOT NULL CHECK ((kind='owner' AND user_role='b2b') OR (kind='sub' AND user_role='b2b_sub')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (user_id, user_role) REFERENCES portal_users(id, role)
        DEFERRABLE INITIALLY DEFERRED,
    UNIQUE (user_id, agency_id, kind)
);
CREATE UNIQUE INDEX portal_agency_one_owner_idx ON portal_agency_memberships(agency_id) WHERE kind='owner';
CREATE INDEX portal_agency_members_idx ON portal_agency_memberships(agency_id, user_id);
-- Circular, deferred FK permits atomic owner + agency + membership creation.
-- It also means removing an owner's membership alone cannot orphan an agency.
ALTER TABLE portal_agencies ADD CONSTRAINT portal_agency_owner_membership_fk
    FOREIGN KEY (owner_user_id, id, owner_kind)
    REFERENCES portal_agency_memberships(user_id, agency_id, kind)
    DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION guard_portal_user_identity() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'portal user identities must be tombstoned, not deleted';
    END IF;
    IF NEW.id <> OLD.id OR NEW.clerk_user_id <> OLD.clerk_user_id OR NEW.created_at <> OLD.created_at THEN
        RAISE EXCEPTION 'portal identity keys are immutable';
    END IF;
    IF OLD.status = 'deleted' AND (NEW.status <> 'deleted' OR NEW.deleted_at IS DISTINCT FROM OLD.deleted_at) THEN
        RAISE EXCEPTION 'deleted portal identities cannot be restored';
    END IF;
    NEW.version := OLD.version + 1;
    NEW.authorization_version := OLD.authorization_version +
        CASE WHEN NEW.role IS DISTINCT FROM OLD.role OR NEW.status IS DISTINCT FROM OLD.status
            OR NEW.authorization_version IS DISTINCT FROM OLD.authorization_version THEN 1 ELSE 0 END;
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER portal_users_guard BEFORE UPDATE OR DELETE ON portal_users
    FOR EACH ROW EXECUTE FUNCTION guard_portal_user_identity();

CREATE FUNCTION guard_portal_agency_identity() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'portal agencies must be archived, not deleted';
    END IF;
    IF NEW.id <> OLD.id OR NEW.agency_code <> OLD.agency_code OR NEW.created_at <> OLD.created_at THEN
        RAISE EXCEPTION 'portal agency keys are immutable';
    END IF;
    IF OLD.status = 'archived' AND NEW.status <> 'archived' THEN
        RAISE EXCEPTION 'archived portal agencies cannot be reactivated';
    END IF;
    NEW.version := OLD.version + 1;
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER portal_agencies_guard BEFORE UPDATE OR DELETE ON portal_agencies
    FOR EACH ROW EXECUTE FUNCTION guard_portal_agency_identity();

CREATE FUNCTION guard_portal_membership_identity() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    subject UUID;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.user_id <> OLD.user_id OR NEW.created_at <> OLD.created_at THEN
            RAISE EXCEPTION 'portal membership subject is immutable';
        END IF;
        NEW.version := OLD.version + 1;
        NEW.updated_at := clock_timestamp();
    END IF;
    subject := CASE WHEN TG_OP='DELETE' THEN OLD.user_id ELSE NEW.user_id END;
    -- Membership changes invalidate previously resolved agency authority too.
    -- The user trigger normalizes the requested invalidation to one increment.
    UPDATE portal_users SET authorization_version=authorization_version+1 WHERE id=subject;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER portal_memberships_guard BEFORE INSERT OR UPDATE OR DELETE ON portal_agency_memberships
    FOR EACH ROW EXECUTE FUNCTION guard_portal_membership_identity();

-- Check the affected user at COMMIT, after all role/membership writes finish.
CREATE FUNCTION check_portal_user_membership() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    subject UUID;
    current_user_row portal_users%ROWTYPE;
BEGIN
    IF TG_TABLE_NAME='portal_users' THEN
        subject := NEW.id;
    ELSE
        subject := CASE WHEN TG_OP='DELETE' THEN OLD.user_id ELSE NEW.user_id END;
    END IF;
    SELECT * INTO current_user_row FROM portal_users WHERE id=subject;
    IF current_user_row.status='active' AND current_user_row.role IN ('b2b','b2b_sub')
       AND NOT EXISTS (SELECT 1 FROM portal_agency_memberships m WHERE m.user_id=subject AND m.user_role=current_user_row.role) THEN
        RAISE EXCEPTION 'active B2B identity requires an agency membership';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER portal_user_membership_required AFTER INSERT OR UPDATE ON portal_users
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION check_portal_user_membership();
CREATE CONSTRAINT TRIGGER portal_membership_user_required AFTER INSERT OR UPDATE OR DELETE ON portal_agency_memberships
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION check_portal_user_membership();

CREATE TABLE portal_identity_audit (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    operation_id UUID NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('operator','user','worker','webhook')),
    actor_id TEXT NOT NULL CHECK (octet_length(actor_id) BETWEEN 1 AND 128),
    action TEXT NOT NULL CHECK (octet_length(action) BETWEEN 1 AND 100),
    target_user_id UUID REFERENCES portal_users(id),
    target_agency_id UUID REFERENCES portal_agencies(id),
    outcome TEXT NOT NULL CHECK (outcome IN ('attempted','succeeded','failed','denied','needs_reconciliation')),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata)='object' AND octet_length(metadata::text)<=8192),
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX portal_identity_audit_operation_idx ON portal_identity_audit(operation_id, id);
CREATE INDEX portal_identity_audit_user_idx ON portal_identity_audit(target_user_id, id DESC);
CREATE TRIGGER portal_identity_audit_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_audit
    FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

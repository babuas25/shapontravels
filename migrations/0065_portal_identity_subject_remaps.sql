-- A provider instance change can give the same reviewed person a new Clerk
-- subject.  Retain the original portal user, role, agency, wallet and business
-- records, and record an explicit one-time subject remap instead of creating a
-- second account or inferring ownership from email/metadata.
CREATE TABLE portal_identity_subject_remaps (
    operation_id UUID PRIMARY KEY,
    target_user_id UUID NOT NULL UNIQUE REFERENCES portal_users(id),
    previous_subject TEXT NOT NULL UNIQUE CHECK (previous_subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
    current_subject TEXT NOT NULL UNIQUE CHECK (current_subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
    operator_subject TEXT NOT NULL CHECK (operator_subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
    mapping_digest TEXT NOT NULL CHECK (mapping_digest ~ '^[a-f0-9]{64}$'),
    provider_evidence_digest TEXT NOT NULL CHECK (provider_evidence_digest ~ '^[a-f0-9]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK (previous_subject <> current_subject)
);
CREATE INDEX portal_identity_subject_remaps_current_idx ON portal_identity_subject_remaps(current_subject);
CREATE TRIGGER portal_identity_subject_remaps_retain
BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_subject_remaps
FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

-- Managed API-client subjects are otherwise immutable. The same durable
-- remap record is required before their live ownership reference may change.
CREATE OR REPLACE FUNCTION enforce_managed_client_access() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF OLD.external_user_id IS NOT NULL AND NEW.external_user_id IS DISTINCT FROM OLD.external_user_id
    AND NOT EXISTS (
        SELECT 1 FROM portal_identity_subject_remaps r
        WHERE r.operation_id::text = current_setting('portal.identity_subject_remap_operation', true)
          AND r.previous_subject = OLD.external_user_id
          AND r.current_subject = NEW.external_user_id
    )
 THEN
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

-- User-owned wallets retain their wallet UUID, ledger and balances. The owner
-- key can move only alongside the same reviewed subject-remap record; agency
-- wallet and account identities remain immutable.
CREATE OR REPLACE FUNCTION protect_wallet_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_TABLE_NAME='wallet_owners' THEN
   IF NEW.id IS DISTINCT FROM OLD.id
      OR NEW.owner_type IS DISTINCT FROM OLD.owner_type
      OR NEW.created_at IS DISTINCT FROM OLD.created_at
      OR (
        NEW.owner_key IS DISTINCT FROM OLD.owner_key AND NOT (
          OLD.owner_type='user' AND NEW.owner_type='user' AND EXISTS (
            SELECT 1 FROM portal_identity_subject_remaps r
            WHERE r.operation_id::text = current_setting('portal.identity_subject_remap_operation', true)
              AND r.previous_subject = OLD.owner_key
              AND r.current_subject = NEW.owner_key
          )
        )
      )
   THEN
    RAISE EXCEPTION 'wallet owner identity is immutable';
   END IF;
 ELSIF TG_TABLE_NAME='wallet_accounts' THEN
   IF
    (NEW.id,NEW.owner_id,NEW.currency,NEW.scale) IS DISTINCT FROM
    (OLD.id,OLD.owner_id,OLD.currency,OLD.scale) THEN
    RAISE EXCEPTION 'wallet account identity is immutable';
   END IF;
 ELSIF TG_TABLE_NAME='wallet_operations' THEN
    IF (NEW.id,NEW.wallet_account_id,NEW.subject_kind,NEW.subject_id,NEW.booking_id,NEW.amount,NEW.currency,NEW.request_hash,NEW.actor_id,NEW.actor_role,NEW.created_at)
       IS DISTINCT FROM
       (OLD.id,OLD.wallet_account_id,OLD.subject_kind,OLD.subject_id,OLD.booking_id,OLD.amount,OLD.currency,OLD.request_hash,OLD.actor_id,OLD.actor_role,OLD.created_at)
       OR (OLD.state IN ('captured','released') AND NEW.state<>OLD.state)
       OR NEW.refunded_amount<OLD.refunded_amount THEN
      RAISE EXCEPTION 'wallet operation identity or terminal state is immutable';
    END IF;
 END IF;
 RETURN NEW;
END;
$$;

-- The ordinary portal-user guard still makes subject changes fail closed.  A
-- remap must first have a permanent evidence row written by the controlled
-- function below in the same transaction.
CREATE OR REPLACE FUNCTION guard_portal_user_identity() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'portal user identities must be tombstoned, not deleted';
    END IF;
    IF NEW.id <> OLD.id OR NEW.created_at <> OLD.created_at THEN
        RAISE EXCEPTION 'portal identity keys are immutable';
    END IF;
    IF NEW.clerk_user_id <> OLD.clerk_user_id AND NOT EXISTS (
        SELECT 1 FROM portal_identity_subject_remaps r
        WHERE r.operation_id::text = current_setting('portal.identity_subject_remap_operation', true)
          AND r.target_user_id = OLD.id
          AND r.previous_subject = OLD.clerk_user_id
          AND r.current_subject = NEW.clerk_user_id
    ) THEN
        RAISE EXCEPTION 'portal identity subjects require an explicit remap';
    END IF;
    IF OLD.status = 'deleted' AND (NEW.status <> 'deleted' OR NEW.deleted_at IS DISTINCT FROM OLD.deleted_at) THEN
        RAISE EXCEPTION 'deleted portal identities cannot be restored';
    END IF;
    NEW.version := OLD.version + 1;
    NEW.authorization_version := OLD.authorization_version +
        CASE WHEN NEW.role IS DISTINCT FROM OLD.role OR NEW.status IS DISTINCT FROM OLD.status
            OR NEW.clerk_user_id IS DISTINCT FROM OLD.clerk_user_id
            OR NEW.authorization_version IS DISTINCT FROM OLD.authorization_version THEN 1 ELSE 0 END;
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END;
$$;

-- This is deliberately database-only: the caller supplies reviewed exact
-- subjects and provider evidence digests. It never accepts an email, display
-- name, metadata role or browser session as mapping evidence.
CREATE FUNCTION portal_identity_apply_subject_remap(
    p_operation_id UUID,
    p_target_user_id UUID,
    p_previous_subject TEXT,
    p_current_subject TEXT,
    p_operator_subject TEXT,
    p_mapping_digest TEXT,
    p_provider_evidence_digest TEXT
) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE
    v_target_user_id UUID;
BEGIN
    IF p_operation_id IS NULL OR p_target_user_id IS NULL
       OR p_previous_subject !~ '^user_[A-Za-z0-9_]{1,123}$'
       OR p_current_subject !~ '^user_[A-Za-z0-9_]{1,123}$'
       OR p_operator_subject !~ '^user_[A-Za-z0-9_]{1,123}$'
       OR p_previous_subject = p_current_subject
       OR p_mapping_digest !~ '^[a-f0-9]{64}$'
       OR p_provider_evidence_digest !~ '^[a-f0-9]{64}$'
    THEN
        RAISE EXCEPTION 'invalid portal identity subject remap';
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('portal_identity_authority', 0));
    SELECT id INTO v_target_user_id
    FROM portal_users
    WHERE clerk_user_id = p_previous_subject
    FOR UPDATE;
    IF v_target_user_id IS NULL OR v_target_user_id <> p_target_user_id THEN
        RAISE EXCEPTION 'previous portal identity subject does not match the reviewed portal account';
    END IF;
    IF EXISTS (SELECT 1 FROM portal_users WHERE clerk_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM portal_identity_subject_remaps WHERE previous_subject = p_previous_subject OR current_subject = p_current_subject OR target_user_id = v_target_user_id)
    THEN
        RAISE EXCEPTION 'portal identity subject remap conflicts with an existing identity';
    END IF;
    IF EXISTS (SELECT 1 FROM api_clients WHERE external_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM wallet_owners WHERE owner_type = 'user' AND owner_key = p_current_subject)
       OR EXISTS (SELECT 1 FROM portal_staff_clients WHERE external_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM portal_hold_drafts WHERE owner_external_user_id = p_current_subject OR creator_external_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM flight_bookings WHERE created_by_external_user_id = p_current_subject)
    THEN
        RAISE EXCEPTION 'current provider subject already has retained business references';
    END IF;

    INSERT INTO portal_identity_subject_remaps(
        operation_id,target_user_id,previous_subject,current_subject,operator_subject,mapping_digest,provider_evidence_digest
    ) VALUES (
        p_operation_id,v_target_user_id,p_previous_subject,p_current_subject,p_operator_subject,p_mapping_digest,p_provider_evidence_digest
    );
    PERFORM set_config('portal.identity_subject_remap_operation', p_operation_id::text, true);

    UPDATE api_clients SET external_user_id = p_current_subject WHERE external_user_id = p_previous_subject;
    UPDATE wallet_owners SET owner_key = p_current_subject WHERE owner_type = 'user' AND owner_key = p_previous_subject;
    UPDATE portal_staff_clients SET external_user_id = p_current_subject WHERE external_user_id = p_previous_subject;
    UPDATE portal_hold_drafts
    SET owner_external_user_id = CASE WHEN owner_external_user_id = p_previous_subject THEN p_current_subject ELSE owner_external_user_id END,
        creator_external_user_id = CASE WHEN creator_external_user_id = p_previous_subject THEN p_current_subject ELSE creator_external_user_id END
    WHERE owner_external_user_id = p_previous_subject OR creator_external_user_id = p_previous_subject;
    UPDATE flight_bookings SET created_by_external_user_id = p_current_subject WHERE created_by_external_user_id = p_previous_subject;
    UPDATE portal_users SET clerk_user_id = p_current_subject WHERE id = v_target_user_id;

    IF EXISTS (SELECT 1 FROM portal_users WHERE clerk_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM api_clients WHERE external_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM wallet_owners WHERE owner_type = 'user' AND owner_key = p_previous_subject)
       OR EXISTS (SELECT 1 FROM portal_staff_clients WHERE external_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM portal_hold_drafts WHERE owner_external_user_id = p_previous_subject OR creator_external_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM flight_bookings WHERE created_by_external_user_id = p_previous_subject)
    THEN
        RAISE EXCEPTION 'portal identity subject remap left retained references behind';
    END IF;

    INSERT INTO portal_identity_audit(
        operation_id,actor_kind,actor_id,action,target_user_id,outcome,metadata
    ) VALUES (
        p_operation_id,'operator',p_operator_subject,'identity.provider_subject_remapped',v_target_user_id,'succeeded',
        jsonb_build_object(
            'mapping_digest',p_mapping_digest,
            'provider_evidence_digest',p_provider_evidence_digest,
            'previous_subject_sha256',encode(sha256(convert_to(p_previous_subject,'UTF8')),'hex'),
            'current_subject_sha256',encode(sha256(convert_to(p_current_subject,'UTF8')),'hex')
        )
    );
END;
$$;

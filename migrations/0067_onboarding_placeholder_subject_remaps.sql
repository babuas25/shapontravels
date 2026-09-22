-- A Clerk instance cutover can provision a new customer/onboarding placeholder
-- before the reviewed retained B2B subject is remapped.  Keep both histories:
-- tombstone only that inert placeholder and move the reviewed retained B2B
-- account to the verified current subject in one controlled transaction.
CREATE TABLE portal_identity_onboarding_placeholder_displacements (
    operation_id UUID PRIMARY KEY REFERENCES portal_identity_subject_remaps(operation_id),
    placeholder_user_id UUID NOT NULL UNIQUE REFERENCES portal_users(id),
    displaced_subject TEXT NOT NULL UNIQUE CHECK (displaced_subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
    tombstone_subject TEXT NOT NULL UNIQUE CHECK (tombstone_subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CHECK (displaced_subject <> tombstone_subject)
);
CREATE TRIGGER portal_identity_onboarding_placeholder_displacements_retain
BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_onboarding_placeholder_displacements
FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

-- A subject remap changes both the portal-user key and any active search
-- control. Defer only this foreign key inside the controlled transaction so
-- neither operation needs an observable intermediary subject.
ALTER TABLE search_user_controls DROP CONSTRAINT search_user_controls_subject_fkey;
ALTER TABLE search_user_controls ADD CONSTRAINT search_user_controls_subject_fkey
    FOREIGN KEY(subject) REFERENCES portal_users(clerk_user_id) DEFERRABLE INITIALLY IMMEDIATE;

-- Both controlled operations use one durable remap record.  A placeholder can
-- change subject only when it is explicitly paired with that exact operation;
-- ordinary portal-user updates remain fail closed.
CREATE OR REPLACE FUNCTION guard_portal_user_identity() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    v_subject_change_authorized BOOLEAN;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'portal user identities must be tombstoned, not deleted';
    END IF;
    IF NEW.id <> OLD.id OR NEW.created_at <> OLD.created_at THEN
        RAISE EXCEPTION 'portal identity keys are immutable';
    END IF;
    IF NEW.clerk_user_id <> OLD.clerk_user_id THEN
        SELECT EXISTS (
            SELECT 1 FROM portal_identity_subject_remaps r
            WHERE r.operation_id::text = current_setting('portal.identity_subject_remap_operation', true)
              AND r.target_user_id = OLD.id
              AND r.previous_subject = OLD.clerk_user_id
              AND r.current_subject = NEW.clerk_user_id
        ) OR EXISTS (
            SELECT 1
            FROM portal_identity_subject_remaps r
            JOIN portal_identity_onboarding_placeholder_displacements d ON d.operation_id=r.operation_id
            WHERE r.operation_id::text = current_setting('portal.identity_subject_remap_operation', true)
              AND d.placeholder_user_id = OLD.id
              AND d.displaced_subject = OLD.clerk_user_id
              AND d.tombstone_subject = NEW.clerk_user_id
        ) INTO v_subject_change_authorized;
        IF NOT v_subject_change_authorized THEN
            RAISE EXCEPTION 'portal identity subjects require an explicit remap';
        END IF;
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

-- Shared subject-key updates are private to the two controlled remap routines.
-- Provider inbox/state and search history retain historical subjects as evidence;
-- live ownership/control rows move with the reviewed current subject.
CREATE FUNCTION portal_identity_move_subject_references(
    p_previous_subject TEXT,
    p_current_subject TEXT
) RETURNS VOID LANGUAGE plpgsql AS $$
BEGIN
    SET CONSTRAINTS search_user_controls_subject_fkey DEFERRED;
    UPDATE api_clients SET external_user_id = p_current_subject WHERE external_user_id = p_previous_subject;
    UPDATE wallet_owners SET owner_key = p_current_subject WHERE owner_type = 'user' AND owner_key = p_previous_subject;
    UPDATE portal_staff_clients SET external_user_id = p_current_subject WHERE external_user_id = p_previous_subject;
    UPDATE portal_hold_drafts
    SET owner_external_user_id = CASE WHEN owner_external_user_id = p_previous_subject THEN p_current_subject ELSE owner_external_user_id END,
        creator_external_user_id = CASE WHEN creator_external_user_id = p_previous_subject THEN p_current_subject ELSE creator_external_user_id END
    WHERE owner_external_user_id = p_previous_subject OR creator_external_user_id = p_previous_subject;
    UPDATE flight_bookings SET created_by_external_user_id = p_current_subject WHERE created_by_external_user_id = p_previous_subject;
    UPDATE search_user_controls SET subject = p_current_subject WHERE subject = p_previous_subject;
END;
$$;
REVOKE ALL ON FUNCTION portal_identity_move_subject_references(TEXT,TEXT) FROM PUBLIC;

-- Add the live search control to the previously introduced normal remap path.
CREATE OR REPLACE FUNCTION portal_identity_apply_subject_remap(
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
       OR EXISTS (SELECT 1 FROM search_user_controls WHERE subject = p_current_subject)
    THEN
        RAISE EXCEPTION 'current provider subject already has retained business references';
    END IF;

    INSERT INTO portal_identity_subject_remaps(
        operation_id,target_user_id,previous_subject,current_subject,operator_subject,mapping_digest,provider_evidence_digest
    ) VALUES (
        p_operation_id,v_target_user_id,p_previous_subject,p_current_subject,p_operator_subject,p_mapping_digest,p_provider_evidence_digest
    );
    PERFORM set_config('portal.identity_subject_remap_operation', p_operation_id::text, true);
    PERFORM portal_identity_move_subject_references(p_previous_subject, p_current_subject);
    UPDATE portal_users SET clerk_user_id = p_current_subject WHERE id = v_target_user_id;

    IF EXISTS (SELECT 1 FROM portal_users WHERE clerk_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM api_clients WHERE external_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM wallet_owners WHERE owner_type = 'user' AND owner_key = p_previous_subject)
       OR EXISTS (SELECT 1 FROM portal_staff_clients WHERE external_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM portal_hold_drafts WHERE owner_external_user_id = p_previous_subject OR creator_external_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM flight_bookings WHERE created_by_external_user_id = p_previous_subject)
       OR EXISTS (SELECT 1 FROM search_user_controls WHERE subject = p_previous_subject)
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
REVOKE ALL ON FUNCTION portal_identity_apply_subject_remap(UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT) FROM PUBLIC;

-- This routine accepts only an exact reviewed B2B mapping.  It does not accept
-- an email, metadata role or browser session.  The duplicate current-subject
-- portal row must be an inert customer/onboarding placeholder; all business
-- references to it are rejected dynamically except immutable audit/mail history.
CREATE FUNCTION portal_identity_apply_onboarding_placeholder_subject_remap(
    p_operation_id UUID,
    p_target_user_id UUID,
    p_placeholder_user_id UUID,
    p_previous_subject TEXT,
    p_current_subject TEXT,
    p_operator_subject TEXT,
    p_mapping_digest TEXT,
    p_provider_evidence_digest TEXT
) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE
    v_target portal_users%ROWTYPE;
    v_placeholder portal_users%ROWTYPE;
    v_tombstone_subject TEXT;
    v_reference RECORD;
    v_reference_exists BOOLEAN;
BEGIN
    IF p_operation_id IS NULL OR p_target_user_id IS NULL OR p_placeholder_user_id IS NULL
       OR p_target_user_id = p_placeholder_user_id
       OR p_previous_subject !~ '^user_[A-Za-z0-9_]{1,123}$'
       OR p_current_subject !~ '^user_[A-Za-z0-9_]{1,123}$'
       OR p_operator_subject !~ '^user_[A-Za-z0-9_]{1,123}$'
       OR p_previous_subject = p_current_subject
       OR p_mapping_digest !~ '^[a-f0-9]{64}$'
       OR p_provider_evidence_digest !~ '^[a-f0-9]{64}$'
    THEN
        RAISE EXCEPTION 'invalid onboarding placeholder subject remap';
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('portal_identity_authority', 0));
    SELECT * INTO v_target FROM portal_users WHERE id = p_target_user_id FOR UPDATE;
    SELECT * INTO v_placeholder FROM portal_users WHERE id = p_placeholder_user_id FOR UPDATE;
    IF NOT FOUND OR v_target.id IS NULL OR v_placeholder.id IS NULL
       OR v_target.clerk_user_id <> p_previous_subject
       OR v_target.role <> 'b2b' OR v_target.status <> 'active'
       OR v_placeholder.clerk_user_id <> p_current_subject
       OR v_placeholder.role <> 'customer' OR v_placeholder.status <> 'onboarding'
    THEN
        RAISE EXCEPTION 'reviewed retained B2B account and onboarding placeholder do not match';
    END IF;
    IF EXISTS (
        SELECT 1 FROM portal_identity_subject_remaps
        WHERE previous_subject IN (p_previous_subject,p_current_subject)
           OR current_subject IN (p_previous_subject,p_current_subject)
           OR target_user_id IN (p_target_user_id,p_placeholder_user_id)
    ) OR EXISTS (
        SELECT 1 FROM portal_identity_onboarding_placeholder_displacements
        WHERE placeholder_user_id = p_placeholder_user_id OR displaced_subject = p_current_subject
    ) THEN
        RAISE EXCEPTION 'onboarding placeholder subject remap conflicts with prior evidence';
    END IF;

    v_tombstone_subject := 'user_retired_' || replace(p_operation_id::text, '-', '');
    IF EXISTS (SELECT 1 FROM portal_users WHERE clerk_user_id = v_tombstone_subject)
       OR EXISTS (SELECT 1 FROM portal_identity_onboarding_placeholder_displacements WHERE tombstone_subject = v_tombstone_subject)
    THEN
        RAISE EXCEPTION 'generated placeholder tombstone subject conflicts';
    END IF;
    IF EXISTS (SELECT 1 FROM api_clients WHERE external_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM wallet_owners WHERE owner_type = 'user' AND owner_key = p_current_subject)
       OR EXISTS (SELECT 1 FROM portal_staff_clients WHERE external_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM portal_hold_drafts WHERE owner_external_user_id = p_current_subject OR creator_external_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM flight_bookings WHERE created_by_external_user_id = p_current_subject)
       OR EXISTS (SELECT 1 FROM search_user_controls WHERE subject = p_current_subject)
    THEN
        RAISE EXCEPTION 'onboarding placeholder current subject has retained business references';
    END IF;
    FOR v_reference IN
        SELECT ns.nspname AS schema_name, cls.relname AS table_name, attr.attname AS column_name
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid=con.conrelid
        JOIN pg_namespace ns ON ns.oid=cls.relnamespace
        JOIN unnest(con.conkey) WITH ORDINALITY AS keys(attnum,ord) ON true
        JOIN unnest(con.confkey) WITH ORDINALITY AS refs(attnum,ord) ON refs.ord=keys.ord
        JOIN pg_attribute attr ON attr.attrelid=con.conrelid AND attr.attnum=keys.attnum
        WHERE con.contype='f' AND con.confrelid='portal_users'::regclass
          AND refs.attnum=(SELECT attnum FROM pg_attribute WHERE attrelid='portal_users'::regclass AND attname='id')
          AND NOT (cls.relname='portal_identity_audit' AND attr.attname='target_user_id')
          AND NOT (cls.relname='portal_identity_mail' AND attr.attname='user_id')
    LOOP
        EXECUTE format('SELECT EXISTS(SELECT 1 FROM %I.%I WHERE %I=$1)', v_reference.schema_name, v_reference.table_name, v_reference.column_name)
        INTO v_reference_exists USING v_placeholder.id;
        IF v_reference_exists THEN
            RAISE EXCEPTION 'onboarding placeholder has retained user reference in %.%', v_reference.table_name, v_reference.column_name;
        END IF;
    END LOOP;
    IF EXISTS (
        SELECT 1 FROM portal_identity_creates
        WHERE user_id=v_placeholder.id AND state NOT IN ('completed','cancelled')
    ) OR EXISTS (
        SELECT 1 FROM portal_identity_invitations
        WHERE user_id=v_placeholder.id AND state NOT IN ('accepted','revoked','cancelled','expired')
    ) THEN
        RAISE EXCEPTION 'onboarding placeholder has unfinished identity work';
    END IF;

    INSERT INTO portal_identity_subject_remaps(
        operation_id,target_user_id,previous_subject,current_subject,operator_subject,mapping_digest,provider_evidence_digest
    ) VALUES (
        p_operation_id,p_target_user_id,p_previous_subject,p_current_subject,p_operator_subject,p_mapping_digest,p_provider_evidence_digest
    );
    INSERT INTO portal_identity_onboarding_placeholder_displacements(
        operation_id,placeholder_user_id,displaced_subject,tombstone_subject
    ) VALUES (
        p_operation_id,p_placeholder_user_id,p_current_subject,v_tombstone_subject
    );
    PERFORM set_config('portal.identity_subject_remap_operation', p_operation_id::text, true);

    UPDATE portal_users
    SET clerk_user_id=v_tombstone_subject,status='deleted',deleted_at=clock_timestamp()
    WHERE id=p_placeholder_user_id;
    PERFORM portal_identity_move_subject_references(p_previous_subject, p_current_subject);
    UPDATE portal_users SET clerk_user_id=p_current_subject WHERE id=p_target_user_id;

    IF EXISTS (SELECT 1 FROM portal_users WHERE clerk_user_id=p_previous_subject)
       OR EXISTS (SELECT 1 FROM api_clients WHERE external_user_id=p_previous_subject)
       OR EXISTS (SELECT 1 FROM wallet_owners WHERE owner_type='user' AND owner_key=p_previous_subject)
       OR EXISTS (SELECT 1 FROM portal_staff_clients WHERE external_user_id=p_previous_subject)
       OR EXISTS (SELECT 1 FROM portal_hold_drafts WHERE owner_external_user_id=p_previous_subject OR creator_external_user_id=p_previous_subject)
       OR EXISTS (SELECT 1 FROM flight_bookings WHERE created_by_external_user_id=p_previous_subject)
       OR EXISTS (SELECT 1 FROM search_user_controls WHERE subject=p_previous_subject)
       OR NOT EXISTS (SELECT 1 FROM portal_users WHERE id=p_target_user_id AND clerk_user_id=p_current_subject AND role='b2b' AND status='active')
       OR NOT EXISTS (SELECT 1 FROM portal_users WHERE id=p_placeholder_user_id AND clerk_user_id=v_tombstone_subject AND status='deleted')
    THEN
        RAISE EXCEPTION 'onboarding placeholder subject remap left inconsistent references';
    END IF;

    INSERT INTO portal_identity_audit(
        operation_id,actor_kind,actor_id,action,target_user_id,outcome,metadata
    ) VALUES
    (
        p_operation_id,'operator',p_operator_subject,'identity.provider_subject_remapped',p_target_user_id,'succeeded',
        jsonb_build_object(
            'mapping_digest',p_mapping_digest,
            'provider_evidence_digest',p_provider_evidence_digest,
            'previous_subject_sha256',encode(sha256(convert_to(p_previous_subject,'UTF8')),'hex'),
            'current_subject_sha256',encode(sha256(convert_to(p_current_subject,'UTF8')),'hex')
        )
    ),
    (
        p_operation_id,'operator',p_operator_subject,'identity.onboarding_placeholder_displaced',p_placeholder_user_id,'succeeded',
        jsonb_build_object(
            'mapping_digest',p_mapping_digest,
            'displaced_subject_sha256',encode(sha256(convert_to(p_current_subject,'UTF8')),'hex'),
            'tombstone_subject_sha256',encode(sha256(convert_to(v_tombstone_subject,'UTF8')),'hex')
        )
    );
END;
$$;
REVOKE ALL ON FUNCTION portal_identity_apply_onboarding_placeholder_subject_remap(UUID,UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT) FROM PUBLIC;

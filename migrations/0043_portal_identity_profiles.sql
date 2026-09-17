-- Additive staged profile storage; does not import or activate legacy data.
CREATE FUNCTION valid_portal_profile_fields(kind TEXT, fields JSONB) RETURNS BOOLEAN
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE item RECORD; allowed TEXT[];
BEGIN
    IF jsonb_typeof(fields) IS DISTINCT FROM 'object' THEN RETURN FALSE; END IF;
    allowed := CASE kind WHEN 'profile' THEN ARRAY[
        'givenName','surname','gender','dateOfBirth','address','nationality','passportNo',
        'passportExpiry','mobile','email','agencyName','agencyLicenseNo','agencyAddress',
        'agencyEmail','agencyMobile','website','facebookPage','bankName','accountName',
        'accountNumber','routingNumber','swiftCode','branchCode']
        WHEN 'staff' THEN ARRAY['designation','email','phone','alternativePhone','address','qualification']
        ELSE ARRAY[]::TEXT[] END;
    FOR item IN SELECT * FROM jsonb_each(fields) LOOP
        IF NOT item.key = ANY(allowed) OR jsonb_typeof(item.value) <> 'string'
           OR char_length(item.value #>> '{}') > 500 THEN RETURN FALSE; END IF;
    END LOOP;
    RETURN TRUE;
END;
$$;
CREATE TABLE portal_identity_profiles (
    user_id UUID NOT NULL REFERENCES portal_users(id),
    kind TEXT NOT NULL CHECK (kind IN ('profile','staff')),
    fields JSONB NOT NULL DEFAULT '{}' CHECK (valid_portal_profile_fields(kind,fields)),
    present BOOLEAN NOT NULL DEFAULT TRUE,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(user_id,kind),
    CHECK (present OR (kind='staff' AND fields='{}'::JSONB))
);
CREATE FUNCTION guard_portal_profile() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'profile versions must not be reset by deletion'; END IF;
    IF NEW.user_id<>OLD.user_id OR NEW.kind<>OLD.kind OR NEW.created_at<>OLD.created_at THEN
        RAISE EXCEPTION 'profile keys are immutable';
    END IF;
    NEW.version := OLD.version+1;
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER portal_profile_guard BEFORE UPDATE OR DELETE ON portal_identity_profiles
    FOR EACH ROW EXECUTE FUNCTION guard_portal_profile();
CREATE TRIGGER portal_profile_no_truncate BEFORE TRUNCATE ON portal_identity_profiles
    FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TABLE portal_identity_profile_mutations (
    operation_id UUID PRIMARY KEY,
    actor_user_id UUID NOT NULL REFERENCES portal_users(id),
    target_user_id UUID NOT NULL REFERENCES portal_users(id),
    kind TEXT NOT NULL CHECK (kind IN ('profile','staff')),
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash)=32),
    previous_version BIGINT NOT NULL CHECK (previous_version >= 0),
    committed_version BIGINT NOT NULL CHECK (committed_version=previous_version+1),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER portal_profile_mutations_immutable BEFORE UPDATE OR DELETE OR TRUNCATE
    ON portal_identity_profile_mutations FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

-- Staged deletion journal only; no provider calls, purge or live activation.
CREATE TABLE portal_identity_deletions (
 id UUID PRIMARY KEY,
 actor_user_id UUID NOT NULL REFERENCES portal_users(id),
 target_user_id UUID NOT NULL UNIQUE REFERENCES portal_users(id),
 clerk_user_id TEXT NOT NULL CHECK(clerk_user_id ~ '^user_[A-Za-z0-9_]{1,123}$'),
 agency_id UUID REFERENCES portal_agencies(id),
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32),
 expected_version BIGINT NOT NULL CHECK(expected_version>0),
 state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared','dispatching','needs_reconciliation','reconciling','provider_confirmed','completed')),
 fence BIGINT NOT NULL DEFAULT 0 CHECK(fence>=0),
 claim_token UUID,
 lease_until TIMESTAMPTZ,
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT CHECK(error_code IN ('IDENTITY_PROVIDER_NOT_SENT','IDENTITY_PROVIDER_OUTCOME_UNKNOWN','IDENTITY_PROVIDER_RETRY_LIMIT')),
 CHECK((state IN ('dispatching','reconciling'))=(claim_token IS NOT NULL AND lease_until IS NOT NULL)),
 CHECK((claim_token IS NULL)=(lease_until IS NULL)),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE portal_identity_deletion_attempts (
 claim_token UUID PRIMARY KEY,
 operation_id UUID NOT NULL REFERENCES portal_identity_deletions(id),
 fence BIGINT NOT NULL CHECK(fence>0),
 kind TEXT NOT NULL CHECK(kind IN ('dispatch','reconcile')),
 outcome TEXT CHECK(outcome IN ('confirmed','not_sent','unknown')),
 started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 finished_at TIMESTAMPTZ,
 UNIQUE(operation_id,fence),
 CHECK((outcome IS NULL)=(finished_at IS NULL))
);
CREATE FUNCTION guard_portal_identity_deletion() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.actor_user_id,NEW.target_user_id,NEW.clerk_user_id,NEW.agency_id,NEW.request_hash,NEW.expected_version,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.actor_user_id,OLD.target_user_id,OLD.clerk_user_id,OLD.agency_id,OLD.request_hash,OLD.expected_version,OLD.created_at)
 OR OLD.state='completed' THEN RAISE EXCEPTION 'immutable identity deletion'; END IF;
 IF NEW.state<>OLD.state AND NOT (
 (OLD.state='prepared' AND NEW.state='dispatching') OR
 (OLD.state='dispatching' AND NEW.state IN ('prepared','needs_reconciliation','provider_confirmed')) OR
 (OLD.state='needs_reconciliation' AND NEW.state='reconciling') OR
 (OLD.state='reconciling' AND NEW.state IN ('needs_reconciliation','provider_confirmed')) OR
 (OLD.state='provider_confirmed' AND NEW.state='completed')) THEN RAISE EXCEPTION 'invalid identity deletion transition'; END IF;
 IF NEW.claim_token IS DISTINCT FROM OLD.claim_token AND NEW.claim_token IS NOT NULL THEN
 IF NEW.fence<>OLD.fence+1 THEN RAISE EXCEPTION 'invalid deletion fence'; END IF;
 ELSIF NEW.fence<>OLD.fence THEN RAISE EXCEPTION 'invalid deletion fence'; END IF;
 NEW.updated_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_deletion_guard BEFORE UPDATE ON portal_identity_deletions FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_deletion();
CREATE FUNCTION guard_portal_identity_deletion_attempt() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.claim_token,NEW.operation_id,NEW.fence,NEW.kind,NEW.started_at) IS DISTINCT FROM (OLD.claim_token,OLD.operation_id,OLD.fence,OLD.kind,OLD.started_at)
 OR OLD.outcome IS NOT NULL OR NEW.outcome IS NULL THEN RAISE EXCEPTION 'immutable deletion attempt'; END IF;
 NEW.finished_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_deletion_attempt_guard BEFORE UPDATE ON portal_identity_deletion_attempts FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_deletion_attempt();
CREATE TRIGGER portal_identity_deletion_retain BEFORE DELETE OR TRUNCATE ON portal_identity_deletions FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_deletion_attempt_retain BEFORE DELETE OR TRUNCATE ON portal_identity_deletion_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
-- Once durable denial starts, ordinary updates cannot reopen this identity.
CREATE FUNCTION guard_portal_deleting_identity() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF OLD.status='deleting' AND NEW.status NOT IN ('deleting','deleted') THEN RAISE EXCEPTION 'deleting identity cannot be restored'; END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER portal_deleting_identity_guard BEFORE UPDATE ON portal_users FOR EACH ROW EXECUTE FUNCTION guard_portal_deleting_identity();

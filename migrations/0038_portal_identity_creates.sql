-- Staged provider-account creation intents. Passwords have no storage column.
ALTER TABLE portal_identity_operations DROP CONSTRAINT portal_identity_operations_action_check;
ALTER TABLE portal_identity_operations ADD CHECK(action IN ('set_role','set_access','provision_agency','reactivate_agency','create_account'));
CREATE TABLE portal_identity_creates (
 id UUID PRIMARY KEY,
 actor_user_id UUID NOT NULL REFERENCES portal_users(id),
 user_id UUID NOT NULL UNIQUE,
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32),
 email TEXT NOT NULL CHECK(octet_length(email) BETWEEN 3 AND 254),
 email_key BYTEA NOT NULL CHECK(octet_length(email_key)=32),
 first_name TEXT NOT NULL CHECK(octet_length(first_name) BETWEEN 1 AND 400),
 last_name TEXT NOT NULL CHECK(octet_length(last_name)<=400),
 role TEXT NOT NULL CHECK(role IN ('superadmin','admin','staff_support','staff_account','staff_media','b2b','b2b_sub','customer')),
 agency_id UUID REFERENCES portal_agencies(id),
 agency_version BIGINT CHECK(agency_version>0),
 CHECK((role='b2b_sub')=(agency_id IS NOT NULL)),
 CHECK((agency_id IS NULL)=(agency_version IS NULL)),
 state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared','dispatching','needs_reconciliation','reconciling','provider_confirmed','completed','cancelled')),
 clerk_user_id TEXT UNIQUE CHECK(clerk_user_id ~ '^user_[A-Za-z0-9_]{1,123}$'),
 CHECK((state IN ('provider_confirmed','completed'))=(clerk_user_id IS NOT NULL)),
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
-- Email is a duplicate-dispatch guard, never a user/agency/financial identity key.
CREATE UNIQUE INDEX portal_identity_create_email_pending ON portal_identity_creates(email_key) WHERE state NOT IN ('completed','cancelled');
CREATE INDEX portal_identity_create_actor ON portal_identity_creates(actor_user_id,created_at,id);
CREATE TABLE portal_identity_create_attempts (
 claim_token UUID PRIMARY KEY,
 operation_id UUID NOT NULL REFERENCES portal_identity_creates(id),
 fence BIGINT NOT NULL CHECK(fence>0),
 kind TEXT NOT NULL CHECK(kind IN ('dispatch','reconcile')),
 outcome TEXT CHECK(outcome IN ('confirmed','not_sent','unknown')),
 started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 finished_at TIMESTAMPTZ,
 UNIQUE(operation_id,fence),
 CHECK((outcome IS NULL)=(finished_at IS NULL))
);
CREATE FUNCTION guard_portal_identity_create() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.actor_user_id,NEW.user_id,NEW.request_hash,NEW.email,NEW.email_key,NEW.first_name,NEW.last_name,NEW.role,NEW.agency_id,NEW.agency_version,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.actor_user_id,OLD.user_id,OLD.request_hash,OLD.email,OLD.email_key,OLD.first_name,OLD.last_name,OLD.role,OLD.agency_id,OLD.agency_version,OLD.created_at)
 OR OLD.state IN ('completed','cancelled') OR (OLD.clerk_user_id IS NOT NULL AND NEW.clerk_user_id IS DISTINCT FROM OLD.clerk_user_id) THEN
 RAISE EXCEPTION 'immutable identity create'; END IF;
 IF NEW.state<>OLD.state AND NOT (
 (OLD.state='prepared' AND NEW.state IN ('dispatching','cancelled')) OR
 (OLD.state='dispatching' AND NEW.state IN ('prepared','needs_reconciliation','provider_confirmed','cancelled')) OR
 (OLD.state='needs_reconciliation' AND NEW.state='reconciling') OR
 (OLD.state='reconciling' AND NEW.state IN ('needs_reconciliation','provider_confirmed')) OR
 (OLD.state='provider_confirmed' AND NEW.state='completed')) THEN RAISE EXCEPTION 'invalid identity create transition'; END IF;
 IF NEW.clerk_user_id IS DISTINCT FROM OLD.clerk_user_id AND NEW.state<>'provider_confirmed' THEN RAISE EXCEPTION 'unconfirmed provider subject'; END IF;
 IF NEW.claim_token IS DISTINCT FROM OLD.claim_token AND NEW.claim_token IS NOT NULL THEN
 IF NEW.fence<>OLD.fence+1 THEN RAISE EXCEPTION 'invalid create fence'; END IF;
 ELSIF NEW.fence<>OLD.fence THEN RAISE EXCEPTION 'invalid create fence'; END IF;
 NEW.updated_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_create_guard BEFORE UPDATE ON portal_identity_creates FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_create();
CREATE FUNCTION guard_portal_identity_create_attempt() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.claim_token,NEW.operation_id,NEW.fence,NEW.kind,NEW.started_at) IS DISTINCT FROM (OLD.claim_token,OLD.operation_id,OLD.fence,OLD.kind,OLD.started_at)
 OR OLD.outcome IS NOT NULL OR NEW.outcome IS NULL THEN RAISE EXCEPTION 'immutable create attempt'; END IF;
 NEW.finished_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_create_attempt_guard BEFORE UPDATE ON portal_identity_create_attempts FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_create_attempt();
CREATE TRIGGER portal_identity_creates_retain BEFORE DELETE OR TRUNCATE ON portal_identity_creates FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_create_attempts_retain BEFORE DELETE OR TRUNCATE ON portal_identity_create_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

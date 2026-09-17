-- First lifecycle slice: local role/access changes and their provider effects.
-- No provider accounts are created/deleted and no live authority is activated.
CREATE TABLE portal_identity_operations (
 id UUID PRIMARY KEY,
 actor_user_id UUID NOT NULL REFERENCES portal_users(id),
 target_user_id UUID NOT NULL REFERENCES portal_users(id),
 action TEXT NOT NULL CHECK(action IN ('set_role','set_access')),
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32),
 expected_version BIGINT NOT NULL CHECK(expected_version>0),
 resulting_version BIGINT NOT NULL CHECK(resulting_version>0),
 resulting_role TEXT NOT NULL CHECK(resulting_role IN ('superadmin','admin','staff_support','staff_account','staff_media','b2b','b2b_sub','customer')),
 resulting_status TEXT NOT NULL CHECK(resulting_status IN ('onboarding','active','suspended','deleting','deleted')),
 state TEXT NOT NULL DEFAULT 'pending_effects' CHECK(state IN ('pending_effects','needs_reconciliation','completed')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX portal_identity_operations_actor_idx ON portal_identity_operations(actor_user_id,created_at DESC,id);
CREATE INDEX portal_identity_operations_target_idx ON portal_identity_operations(target_user_id,created_at DESC,id);
CREATE TABLE portal_identity_effects (
 id UUID PRIMARY KEY,
 sequence BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
 operation_id UUID NOT NULL REFERENCES portal_identity_operations(id),
 target_user_id UUID NOT NULL REFERENCES portal_users(id),
 kind TEXT NOT NULL CHECK(kind IN ('revoke_sessions','mirror_metadata')),
 authorization_version BIGINT NOT NULL CHECK(authorization_version>0),
 role TEXT NOT NULL CHECK(role IN ('superadmin','admin','staff_support','staff_account','staff_media','b2b','b2b_sub','customer')),
 status TEXT NOT NULL CHECK(status IN ('onboarding','active','suspended','deleting','deleted')),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','dispatching','retryable','needs_reconciliation','reconciling','confirmed','superseded')),
 fence BIGINT NOT NULL DEFAULT 0 CHECK(fence>=0),
 claim_token UUID,
 lease_until TIMESTAMPTZ,
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0),
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT CHECK(error_code IN ('IDENTITY_PROVIDER_NOT_SENT','IDENTITY_PROVIDER_OUTCOME_UNKNOWN','IDENTITY_PROVIDER_RETRY_LIMIT')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(operation_id,target_user_id,kind),
 CHECK((state IN ('dispatching','reconciling'))=(claim_token IS NOT NULL AND lease_until IS NOT NULL)),
 CHECK((claim_token IS NULL)=(lease_until IS NULL))
);
CREATE INDEX portal_identity_effects_queue_idx ON portal_identity_effects(state,next_attempt_at,sequence);
CREATE INDEX portal_identity_effects_subject_idx ON portal_identity_effects(target_user_id,sequence);
CREATE TABLE portal_identity_effect_attempts (
 claim_token UUID PRIMARY KEY,
 effect_id UUID NOT NULL REFERENCES portal_identity_effects(id),
 fence BIGINT NOT NULL CHECK(fence>0),
 worker_id TEXT NOT NULL CHECK(worker_id ~ '^[A-Za-z0-9_.:-]{1,128}$'),
 kind TEXT NOT NULL CHECK(kind IN ('dispatch','reconcile')),
 outcome TEXT CHECK(outcome IN ('confirmed','not_sent','unknown')),
 started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 finished_at TIMESTAMPTZ,
 UNIQUE(effect_id,fence),
 CHECK((outcome IS NULL)=(finished_at IS NULL))
);
CREATE FUNCTION guard_portal_identity_operation() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.actor_user_id,NEW.target_user_id,NEW.action,NEW.request_hash,NEW.expected_version,NEW.resulting_version,NEW.resulting_role,NEW.resulting_status,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.actor_user_id,OLD.target_user_id,OLD.action,OLD.request_hash,OLD.expected_version,OLD.resulting_version,OLD.resulting_role,OLD.resulting_status,OLD.created_at)
 OR (OLD.state='completed' AND NEW.state<>'completed') THEN RAISE EXCEPTION 'immutable identity operation'; END IF;
 NEW.updated_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_operation_guard BEFORE UPDATE ON portal_identity_operations FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_operation();
CREATE FUNCTION guard_portal_identity_effect() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.sequence,NEW.operation_id,NEW.target_user_id,NEW.kind,NEW.authorization_version,NEW.role,NEW.status,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.sequence,OLD.operation_id,OLD.target_user_id,OLD.kind,OLD.authorization_version,OLD.role,OLD.status,OLD.created_at)
 OR OLD.state IN ('confirmed','superseded') THEN RAISE EXCEPTION 'immutable identity effect'; END IF;
 IF NEW.state<>OLD.state AND NOT (
   (OLD.state IN ('pending','retryable') AND NEW.state IN ('dispatching','superseded')) OR
   (OLD.state='dispatching' AND NEW.state IN ('confirmed','retryable','needs_reconciliation')) OR
   (OLD.state='needs_reconciliation' AND NEW.state='reconciling') OR
   (OLD.state='reconciling' AND NEW.state IN ('confirmed','needs_reconciliation'))
 ) THEN RAISE EXCEPTION 'invalid identity effect transition'; END IF;
 IF NEW.claim_token IS DISTINCT FROM OLD.claim_token AND NEW.claim_token IS NOT NULL THEN
   IF NEW.fence<>OLD.fence+1 THEN RAISE EXCEPTION 'invalid identity fence'; END IF;
 ELSIF NEW.fence<>OLD.fence THEN RAISE EXCEPTION 'invalid identity fence'; END IF;
 NEW.updated_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_effect_guard BEFORE UPDATE ON portal_identity_effects FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_effect();
CREATE FUNCTION guard_portal_identity_attempt() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.claim_token,NEW.effect_id,NEW.fence,NEW.worker_id,NEW.kind,NEW.started_at)
 IS DISTINCT FROM (OLD.claim_token,OLD.effect_id,OLD.fence,OLD.worker_id,OLD.kind,OLD.started_at)
 OR OLD.outcome IS NOT NULL OR NEW.outcome IS NULL THEN RAISE EXCEPTION 'immutable identity attempt'; END IF;
 NEW.finished_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_attempt_guard BEFORE UPDATE ON portal_identity_effect_attempts FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_attempt();
CREATE TRIGGER portal_identity_operations_retain BEFORE DELETE OR TRUNCATE ON portal_identity_operations FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_effects_retain BEFORE DELETE OR TRUNCATE ON portal_identity_effects FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_attempts_retain BEFORE DELETE OR TRUNCATE ON portal_identity_effect_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

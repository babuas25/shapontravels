-- An activation never reverts to legacy authority. No row is seeded here.
CREATE TABLE portal_identity_rollout (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 id UUID NOT NULL UNIQUE,
 target_id TEXT NOT NULL CHECK(target_id ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
 revision BIGINT NOT NULL CHECK(revision>0),
 state TEXT NOT NULL CHECK(state IN ('active','paused')),
 backend_release TEXT NOT NULL CHECK(backend_release ~ '^[a-f0-9]{64}$'),
 frontend_release TEXT NOT NULL CHECK(frontend_release ~ '^[a-f0-9]{64}$'),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE portal_identity_rollout_events (
 rollout_id UUID NOT NULL REFERENCES portal_identity_rollout(id),
 revision BIGINT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('active','paused')),
 operator_user_id UUID NOT NULL REFERENCES portal_users(id),
 backend_release TEXT NOT NULL,
 frontend_release TEXT NOT NULL,
 evidence JSONB NOT NULL CHECK(jsonb_typeof(evidence)='object'),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(rollout_id,revision)
);
CREATE FUNCTION guard_identity_rollout() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.singleton,NEW.id,NEW.target_id,NEW.created_at) IS DISTINCT FROM (OLD.singleton,OLD.id,OLD.target_id,OLD.created_at)
 OR NEW.revision<>OLD.revision+1 OR NEW.state=OLD.state
 OR (NEW.state='paused' AND (NEW.backend_release,NEW.frontend_release) IS DISTINCT FROM (OLD.backend_release,OLD.frontend_release))
 THEN RAISE EXCEPTION 'invalid identity rollout transition'; END IF;
 NEW.updated_at:=clock_timestamp(); RETURN NEW;
END $$;
CREATE TRIGGER portal_identity_rollout_guard BEFORE UPDATE ON portal_identity_rollout FOR EACH ROW EXECUTE FUNCTION guard_identity_rollout();
CREATE TRIGGER portal_identity_rollout_retain BEFORE DELETE OR TRUNCATE ON portal_identity_rollout FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_rollout_events_retain BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_rollout_events FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE FUNCTION check_identity_rollout_event() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM portal_identity_rollout_events e WHERE e.rollout_id=NEW.id AND e.revision=NEW.revision AND e.state=NEW.state AND e.backend_release=NEW.backend_release AND e.frontend_release=NEW.frontend_release)
 THEN RAISE EXCEPTION 'identity rollout requires matching immutable evidence'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER portal_identity_rollout_evidence AFTER INSERT OR UPDATE ON portal_identity_rollout DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION check_identity_rollout_event();

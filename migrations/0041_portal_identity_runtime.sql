-- Durable runtime evidence; no external dispatch or activation occurs here.
CREATE TABLE portal_identity_inbox (
 sequence BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
 event_id TEXT PRIMARY KEY CHECK(event_id ~ '^[A-Za-z0-9_-]{1,128}$'),
 payload_hash BYTEA NOT NULL CHECK(octet_length(payload_hash)=32),
 subject TEXT NOT NULL CHECK(subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
 kind TEXT NOT NULL CHECK(kind IN ('user.created','user.updated','user.deleted')),
 occurred_at BIGINT NOT NULL CHECK(occurred_at>0),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','processing','completed','retry','dead_letter')),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
 fence BIGINT NOT NULL DEFAULT 0 CHECK(fence>=0),
 claim_token UUID,
 lease_until TIMESTAMPTZ,
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT CHECK(error_code IN ('IDENTITY_PROVIDER_UNAVAILABLE','IDENTITY_EVENT_REVIEW_REQUIRED','IDENTITY_EVENT_LEASE_EXPIRED')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK((state='processing')=(claim_token IS NOT NULL AND lease_until IS NOT NULL)),
 CHECK((claim_token IS NULL)=(lease_until IS NULL))
);
CREATE TABLE portal_identity_provider_state (
 subject TEXT PRIMARY KEY CHECK(subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
 deleted BOOLEAN NOT NULL DEFAULT false,
 observed_at BIGINT NOT NULL CHECK(observed_at>0),
 event_id TEXT NOT NULL REFERENCES portal_identity_inbox(event_id)
);
CREATE FUNCTION guard_identity_provider_state() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.subject<>OLD.subject OR (OLD.deleted AND NOT NEW.deleted) OR NEW.observed_at<OLD.observed_at THEN RAISE EXCEPTION 'provider state cannot regress'; END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_provider_state_guard BEFORE UPDATE ON portal_identity_provider_state FOR EACH ROW EXECUTE FUNCTION guard_identity_provider_state();
CREATE TRIGGER portal_identity_provider_state_retain BEFORE DELETE OR TRUNCATE ON portal_identity_provider_state FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE FUNCTION guard_identity_inbox() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.sequence,NEW.event_id,NEW.payload_hash,NEW.subject,NEW.kind,NEW.occurred_at,NEW.created_at) IS DISTINCT FROM
 (OLD.sequence,OLD.event_id,OLD.payload_hash,OLD.subject,OLD.kind,OLD.occurred_at,OLD.created_at) OR OLD.state='completed' THEN RAISE EXCEPTION 'immutable inbox evidence'; END IF;
 IF NEW.claim_token IS DISTINCT FROM OLD.claim_token AND NEW.claim_token IS NOT NULL THEN
 IF NEW.fence<>OLD.fence+1 THEN RAISE EXCEPTION 'invalid inbox fence'; END IF;
 ELSIF NEW.fence<>OLD.fence THEN RAISE EXCEPTION 'invalid inbox fence'; END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_inbox_guard BEFORE UPDATE ON portal_identity_inbox FOR EACH ROW EXECUTE FUNCTION guard_identity_inbox();
CREATE TRIGGER portal_identity_inbox_retain BEFORE DELETE OR TRUNCATE ON portal_identity_inbox FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE INDEX portal_identity_inbox_pending ON portal_identity_inbox(state,next_attempt_at,sequence);

CREATE TABLE portal_identity_mail (
 id UUID PRIMARY KEY,
 sequence BIGINT GENERATED ALWAYS AS IDENTITY UNIQUE,
 kind TEXT NOT NULL CHECK(kind IN ('invitation','account','welcome')),
 audience TEXT NOT NULL CHECK(audience IN ('recipient','archive')),
 invitation_id UUID REFERENCES portal_identity_invitations(id),
 user_id UUID REFERENCES portal_users(id),
 CHECK((kind='invitation')=(invitation_id IS NOT NULL)),
 CHECK((kind<>'invitation')=(user_id IS NOT NULL)),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','sending','sent','unknown','blocked')),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
 fence BIGINT NOT NULL DEFAULT 0 CHECK(fence>=0),
 claim_token UUID,
 lease_until TIMESTAMPTZ,
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT CHECK(error_code IN ('IDENTITY_MAIL_NOT_SENT','IDENTITY_MAIL_UNKNOWN','IDENTITY_MAIL_RETRY_LIMIT')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK((state='sending')=(claim_token IS NOT NULL AND lease_until IS NOT NULL)),
 CHECK((claim_token IS NULL)=(lease_until IS NULL)),
 UNIQUE(kind,invitation_id,audience), UNIQUE(kind,user_id,audience)
);
CREATE TABLE portal_identity_mail_attempts (
 claim_token UUID PRIMARY KEY,
 mail_id UUID NOT NULL REFERENCES portal_identity_mail(id),
 fence BIGINT NOT NULL CHECK(fence>0),
 transport_started BOOLEAN NOT NULL DEFAULT false,
 outcome TEXT CHECK(outcome IN ('sent','not_sent','unknown')),
 started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 finished_at TIMESTAMPTZ,
 UNIQUE(mail_id,fence), CHECK((outcome IS NULL)=(finished_at IS NULL))
);
CREATE FUNCTION guard_identity_mail() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.sequence,NEW.kind,NEW.audience,NEW.invitation_id,NEW.user_id,NEW.created_at) IS DISTINCT FROM
 (OLD.id,OLD.sequence,OLD.kind,OLD.audience,OLD.invitation_id,OLD.user_id,OLD.created_at) OR OLD.state IN ('sent','blocked') THEN RAISE EXCEPTION 'immutable mail evidence'; END IF;
 IF OLD.state='unknown' AND NEW.state<>'unknown' THEN RAISE EXCEPTION 'ambiguous mail cannot be resent'; END IF;
 IF NEW.state<>OLD.state AND NOT ((OLD.state='pending' AND NEW.state IN ('sending','blocked')) OR (OLD.state='sending' AND NEW.state IN ('sent','pending','unknown','blocked'))) THEN RAISE EXCEPTION 'invalid mail transition'; END IF;
 IF NEW.claim_token IS DISTINCT FROM OLD.claim_token AND NEW.claim_token IS NOT NULL THEN
 IF NEW.fence<>OLD.fence+1 THEN RAISE EXCEPTION 'invalid mail fence'; END IF;
 ELSIF NEW.fence<>OLD.fence THEN RAISE EXCEPTION 'invalid mail fence'; END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_mail_guard BEFORE UPDATE ON portal_identity_mail FOR EACH ROW EXECUTE FUNCTION guard_identity_mail();
CREATE FUNCTION guard_identity_mail_attempt() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.claim_token,NEW.mail_id,NEW.fence,NEW.started_at) IS DISTINCT FROM (OLD.claim_token,OLD.mail_id,OLD.fence,OLD.started_at) OR OLD.outcome IS NOT NULL OR (OLD.transport_started AND NOT NEW.transport_started) OR (NEW.outcome IS NULL AND (OLD.transport_started OR NOT NEW.transport_started)) THEN RAISE EXCEPTION 'immutable mail attempt'; END IF;
 IF NEW.outcome IS NOT NULL THEN NEW.finished_at:=clock_timestamp(); END IF; RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_mail_attempt_guard BEFORE UPDATE ON portal_identity_mail_attempts FOR EACH ROW EXECUTE FUNCTION guard_identity_mail_attempt();
CREATE TRIGGER portal_identity_mail_retain BEFORE DELETE OR TRUNCATE ON portal_identity_mail FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_mail_attempt_retain BEFORE DELETE OR TRUNCATE ON portal_identity_mail_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE INDEX portal_identity_mail_pending ON portal_identity_mail(state,next_attempt_at,sequence);

ALTER TABLE portal_identity_invitations DROP CONSTRAINT portal_identity_invitations_state_check;
ALTER TABLE portal_identity_invitations ADD CHECK(state IN ('prepared','issuing','issue_unknown','observing_issue','pending','revoking','revoke_unknown','observing_revoke','revoked','accepted','cancelled','expired'));
ALTER TABLE portal_identity_invitation_attempts DROP CONSTRAINT portal_identity_invitation_attempts_provider_state_check;
ALTER TABLE portal_identity_invitation_attempts ADD CHECK(provider_state IN ('pending','accepted','revoked','expired'));
DROP INDEX portal_identity_invitation_email_pending;
CREATE UNIQUE INDEX portal_identity_invitation_email_pending ON portal_identity_invitations(email_key) WHERE state NOT IN ('accepted','revoked','cancelled','expired');
CREATE OR REPLACE FUNCTION guard_portal_identity_invitation() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.issuer_user_id,NEW.user_id,NEW.request_hash,NEW.email,NEW.email_key,NEW.role,NEW.agency_id,NEW.agency_version,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.issuer_user_id,OLD.user_id,OLD.request_hash,OLD.email,OLD.email_key,OLD.role,OLD.agency_id,OLD.agency_version,OLD.created_at)
 OR OLD.state IN ('accepted','revoked','cancelled','expired') OR (OLD.revocation_requested AND NOT NEW.revocation_requested)
 OR (OLD.provider_id IS NOT NULL AND NEW.provider_id IS DISTINCT FROM OLD.provider_id) THEN RAISE EXCEPTION 'immutable invitation'; END IF;
 IF NEW.state<>OLD.state AND NOT (
 (OLD.state='prepared' AND NEW.state IN ('issuing','cancelled')) OR
 (OLD.state='issuing' AND NEW.state IN ('prepared','issue_unknown','pending','revoked','expired','cancelled')) OR
 (OLD.state='issue_unknown' AND NEW.state='observing_issue') OR
 (OLD.state='observing_issue' AND NEW.state IN ('issue_unknown','pending','revoked','expired')) OR
 (OLD.state='pending' AND NEW.state IN ('revoking','accepted','revoked','expired')) OR
 (OLD.state='revoking' AND NEW.state IN ('pending','revoke_unknown','revoked','expired')) OR
 (OLD.state='revoke_unknown' AND NEW.state='observing_revoke') OR
 (OLD.state='observing_revoke' AND NEW.state IN ('revoke_unknown','revoked','expired'))
 ) THEN RAISE EXCEPTION 'invalid invitation transition'; END IF;
 IF NEW.state='accepted' AND NEW.revocation_requested THEN RAISE EXCEPTION 'revocation blocks acceptance'; END IF;
 IF NEW.claim_token IS DISTINCT FROM OLD.claim_token AND NEW.claim_token IS NOT NULL THEN
 IF NEW.fence<>OLD.fence+1 THEN RAISE EXCEPTION 'invalid invitation fence'; END IF;
 ELSIF NEW.fence<>OLD.fence THEN RAISE EXCEPTION 'invalid invitation fence'; END IF;
 NEW.version:=OLD.version+1; NEW.updated_at:=clock_timestamp(); RETURN NEW;
END; $$;

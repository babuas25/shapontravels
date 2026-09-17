-- Staged invitations. No emails/provider writes are activated by this migration.
ALTER TABLE portal_identity_operations DROP CONSTRAINT portal_identity_operations_action_check;
ALTER TABLE portal_identity_operations ADD CHECK(action IN ('set_role','set_access','provision_agency','reactivate_agency','create_account','accept_invitation'));
CREATE TABLE portal_identity_invitations (
 id UUID PRIMARY KEY,
 issuer_user_id UUID NOT NULL REFERENCES portal_users(id),
 user_id UUID NOT NULL UNIQUE,
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32),
 email TEXT NOT NULL CHECK(octet_length(email) BETWEEN 3 AND 254),
 email_key BYTEA NOT NULL CHECK(octet_length(email_key)=32),
 role TEXT NOT NULL CHECK(role IN ('superadmin','admin','staff_support','staff_account','staff_media','b2b','b2b_sub','customer')),
 agency_id UUID REFERENCES portal_agencies(id),
 agency_version BIGINT CHECK(agency_version>0),
 CHECK((role='b2b_sub')=(agency_id IS NOT NULL)),
 CHECK((agency_id IS NULL)=(agency_version IS NULL)),
 state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared','issuing','issue_unknown','observing_issue','pending','revoking','revoke_unknown','observing_revoke','revoked','accepted','cancelled')),
 provider_id TEXT UNIQUE CHECK(provider_id ~ '^[A-Za-z0-9_-]{1,128}$'),
 CHECK(state NOT IN ('pending','revoking','revoke_unknown','observing_revoke','revoked','accepted') OR provider_id IS NOT NULL),
 accepted_subject TEXT UNIQUE CHECK(accepted_subject ~ '^user_[A-Za-z0-9_]{1,123}$'),
 CHECK((state='accepted')=(accepted_subject IS NOT NULL)),
 revocation_requested BOOLEAN NOT NULL DEFAULT FALSE,
 version BIGINT NOT NULL DEFAULT 1 CHECK(version>0),
 fence BIGINT NOT NULL DEFAULT 0 CHECK(fence>=0),
 claim_token UUID,
 lease_until TIMESTAMPTZ,
 issue_attempts INTEGER NOT NULL DEFAULT 0 CHECK(issue_attempts BETWEEN 0 AND 5),
 revoke_attempts INTEGER NOT NULL DEFAULT 0 CHECK(revoke_attempts BETWEEN 0 AND 5),
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT CHECK(error_code IN ('IDENTITY_PROVIDER_NOT_SENT','IDENTITY_PROVIDER_OUTCOME_UNKNOWN','IDENTITY_PROVIDER_RETRY_LIMIT')),
 CHECK((state IN ('issuing','observing_issue','revoking','observing_revoke'))=(claim_token IS NOT NULL AND lease_until IS NOT NULL)),
 CHECK((claim_token IS NULL)=(lease_until IS NULL)),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX portal_identity_invitation_email_pending ON portal_identity_invitations(email_key) WHERE state NOT IN ('accepted','revoked','cancelled');
CREATE INDEX portal_identity_invitation_agency ON portal_identity_invitations(agency_id,state);
CREATE INDEX portal_identity_invitation_issuer ON portal_identity_invitations(issuer_user_id,created_at,id);
CREATE TABLE portal_identity_invitation_attempts (
 claim_token UUID PRIMARY KEY,
 invitation_id UUID NOT NULL REFERENCES portal_identity_invitations(id),
 fence BIGINT NOT NULL CHECK(fence>0),
 kind TEXT NOT NULL CHECK(kind IN ('issue','observe_issue','revoke','observe_revoke')),
 outcome TEXT CHECK(outcome IN ('confirmed','not_sent','unknown')),
 provider_state TEXT CHECK(provider_state IN ('pending','accepted','revoked')),
 CHECK((outcome IS NOT DISTINCT FROM 'confirmed')=(provider_state IS NOT NULL)),
 started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 finished_at TIMESTAMPTZ,
 UNIQUE(invitation_id,fence),
 CHECK((outcome IS NULL)=(finished_at IS NULL))
);
-- Intent for the existing branded delivery transport. No bearer acceptance URL
-- or invitation ticket is persisted; the future adapter retrieves it server-side.
CREATE TABLE portal_identity_invitation_mail (
 invitation_id UUID PRIMARY KEY REFERENCES portal_identity_invitations(id),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','blocked')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION guard_portal_identity_invitation() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.issuer_user_id,NEW.user_id,NEW.request_hash,NEW.email,NEW.email_key,NEW.role,NEW.agency_id,NEW.agency_version,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.issuer_user_id,OLD.user_id,OLD.request_hash,OLD.email,OLD.email_key,OLD.role,OLD.agency_id,OLD.agency_version,OLD.created_at)
 OR OLD.state IN ('accepted','revoked','cancelled') OR (OLD.revocation_requested AND NOT NEW.revocation_requested)
 OR (OLD.provider_id IS NOT NULL AND NEW.provider_id IS DISTINCT FROM OLD.provider_id) THEN RAISE EXCEPTION 'immutable invitation'; END IF;
 IF NEW.state<>OLD.state AND NOT (
 (OLD.state='prepared' AND NEW.state IN ('issuing','cancelled')) OR
 (OLD.state='issuing' AND NEW.state IN ('prepared','issue_unknown','pending','revoked','cancelled')) OR
 (OLD.state='issue_unknown' AND NEW.state='observing_issue') OR
 (OLD.state='observing_issue' AND NEW.state IN ('issue_unknown','pending','revoked')) OR
 (OLD.state='pending' AND NEW.state IN ('revoking','accepted','revoked')) OR
 (OLD.state='revoking' AND NEW.state IN ('pending','revoke_unknown','revoked')) OR
 (OLD.state='revoke_unknown' AND NEW.state='observing_revoke') OR
 (OLD.state='observing_revoke' AND NEW.state IN ('revoke_unknown','revoked'))
 ) THEN RAISE EXCEPTION 'invalid invitation transition'; END IF;
 IF NEW.state='accepted' AND NEW.revocation_requested THEN RAISE EXCEPTION 'revocation blocks acceptance'; END IF;
 IF NEW.claim_token IS DISTINCT FROM OLD.claim_token AND NEW.claim_token IS NOT NULL THEN
 IF NEW.fence<>OLD.fence+1 THEN RAISE EXCEPTION 'invalid invitation fence'; END IF;
 ELSIF NEW.fence<>OLD.fence THEN RAISE EXCEPTION 'invalid invitation fence'; END IF;
 NEW.version:=OLD.version+1; NEW.updated_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_invitation_guard BEFORE UPDATE ON portal_identity_invitations FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_invitation();
CREATE FUNCTION guard_portal_identity_invitation_attempt() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.claim_token,NEW.invitation_id,NEW.fence,NEW.kind,NEW.started_at) IS DISTINCT FROM (OLD.claim_token,OLD.invitation_id,OLD.fence,OLD.kind,OLD.started_at)
 OR OLD.outcome IS NOT NULL OR NEW.outcome IS NULL THEN RAISE EXCEPTION 'immutable invitation attempt'; END IF;
 NEW.finished_at:=clock_timestamp(); RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_invitation_attempt_guard BEFORE UPDATE ON portal_identity_invitation_attempts FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_invitation_attempt();
CREATE FUNCTION guard_portal_identity_invitation_mail() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.invitation_id,NEW.created_at) IS DISTINCT FROM (OLD.invitation_id,OLD.created_at) OR OLD.state='blocked' THEN RAISE EXCEPTION 'immutable invitation mail intent'; END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_invitation_mail_guard BEFORE UPDATE ON portal_identity_invitation_mail FOR EACH ROW EXECUTE FUNCTION guard_portal_identity_invitation_mail();
CREATE TRIGGER portal_identity_invitation_retain BEFORE DELETE OR TRUNCATE ON portal_identity_invitations FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_invitation_attempt_retain BEFORE DELETE OR TRUNCATE ON portal_identity_invitation_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_invitation_mail_retain BEFORE DELETE OR TRUNCATE ON portal_identity_invitation_mail FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

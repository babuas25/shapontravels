-- One acknowledgement per committed application submission. Existing mail and
-- attempt history is retained; this does not enqueue historical applications.
ALTER TABLE portal_identity_mail DROP CONSTRAINT portal_identity_mail_kind_check;
ALTER TABLE portal_identity_mail ADD CONSTRAINT portal_identity_mail_kind_check
 CHECK(kind IN ('invitation','account','welcome','b2b_activated','application_submitted'));
ALTER TABLE portal_identity_mail ADD COLUMN application_version BIGINT;
ALTER TABLE portal_identity_mail ADD CONSTRAINT portal_identity_mail_application_history_fkey
 FOREIGN KEY(user_id,application_version) REFERENCES portal_identity_application_history(user_id,version);
ALTER TABLE portal_identity_mail ADD CONSTRAINT portal_identity_mail_application_version_check
 CHECK((kind='application_submitted' AND application_version IS NOT NULL AND application_version>0)
    OR (kind<>'application_submitted' AND application_version IS NULL));
ALTER TABLE portal_identity_mail DROP CONSTRAINT portal_identity_mail_kind_user_id_audience_key;
CREATE UNIQUE INDEX portal_identity_mail_user_once ON portal_identity_mail(kind,user_id,audience)
 WHERE kind<>'application_submitted';
CREATE UNIQUE INDEX portal_identity_mail_submission_once ON portal_identity_mail(user_id,application_version,audience)
 WHERE kind='application_submitted';

CREATE FUNCTION guard_identity_mail_application_version() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.application_version IS DISTINCT FROM OLD.application_version THEN
  RAISE EXCEPTION 'immutable application mail version';
 END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER portal_identity_mail_application_version_guard BEFORE UPDATE ON portal_identity_mail
 FOR EACH ROW EXECUTE FUNCTION guard_identity_mail_application_version();

-- Additive application, document and sub-user parity. No import or live activation.
ALTER TABLE portal_identity_operations DROP CONSTRAINT portal_identity_operations_action_check;
ALTER TABLE portal_identity_operations ADD CHECK(action IN ('set_role','set_access','provision_agency','reactivate_agency','create_account','accept_invitation','approve_application','rename_sub_user'));
ALTER TABLE portal_identity_effects DROP CONSTRAINT portal_identity_effects_kind_check;
ALTER TABLE portal_identity_effects ADD CHECK(kind IN ('revoke_sessions','mirror_metadata','mirror_name'));
CREATE TABLE portal_identity_name_effects (
 effect_id UUID PRIMARY KEY REFERENCES portal_identity_effects(id),
 first_name TEXT NOT NULL CHECK(char_length(first_name)<=60),last_name TEXT NOT NULL CHECK(char_length(last_name)<=60)
);
CREATE TRIGGER portal_identity_name_effects_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_name_effects FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
ALTER TABLE portal_identity_mail DROP CONSTRAINT portal_identity_mail_kind_check;
ALTER TABLE portal_identity_mail ADD CHECK(kind IN ('invitation','account','welcome','b2b_activated'));

CREATE FUNCTION valid_identity_application_fields(fields JSONB) RETURNS BOOLEAN LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE item RECORD; required TEXT[]:=ARRAY['agencyName','businessMobile','businessEmail','businessAddress','fullName','businessType','personalMobile','personalAddress'];
BEGIN
 IF jsonb_typeof(fields)<>'object' OR NOT fields ?& required OR (SELECT count(*) FROM jsonb_object_keys(fields))<>8 THEN RETURN FALSE; END IF;
 FOR item IN SELECT * FROM jsonb_each(fields) LOOP
  IF NOT item.key=ANY(required) OR jsonb_typeof(item.value)<>'string' OR char_length(btrim(item.value #>> '{}')) NOT BETWEEN 1 AND 500 THEN RETURN FALSE; END IF;
 END LOOP;
 RETURN fields->>'businessType' IN ('proprietor','partner');
END $$;
CREATE TABLE portal_identity_applications (
 user_id UUID PRIMARY KEY REFERENCES portal_users(id),
 version BIGINT NOT NULL DEFAULT 1 CHECK(version>0),
 status TEXT NOT NULL CHECK(status IN ('pending','accepted','rejected')),
 fields JSONB NOT NULL CHECK(valid_identity_application_fields(fields) AND octet_length(fields::text)<=24000),
 documents UUID[] NOT NULL DEFAULT '{}' CHECK(cardinality(documents)<=5),
 reviewer_id UUID REFERENCES portal_users(id),
 review_note TEXT CHECK(char_length(review_note)<=500),
 reviewed_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK((status='pending')=(reviewer_id IS NULL AND reviewed_at IS NULL)),
 CHECK((reviewer_id IS NULL)=(reviewed_at IS NULL))
);
CREATE INDEX portal_identity_application_pending ON portal_identity_applications(status,user_id);
CREATE FUNCTION guard_identity_application() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'retain application history'; END IF;
 IF NEW.user_id<>OLD.user_id OR NEW.created_at<>OLD.created_at OR NOT (
 (OLD.status='pending' AND NEW.status IN ('accepted','rejected') AND NEW.fields=OLD.fields AND NEW.documents=OLD.documents)
 OR (OLD.status='rejected' AND NEW.status='pending')) THEN RAISE EXCEPTION 'invalid application transition'; END IF;
 NEW.version:=OLD.version+1; NEW.updated_at:=clock_timestamp(); RETURN NEW;
END $$;
CREATE TRIGGER portal_identity_application_guard BEFORE UPDATE OR DELETE ON portal_identity_applications FOR EACH ROW EXECUTE FUNCTION guard_identity_application();
CREATE TABLE portal_identity_phase5_commands (
 id UUID PRIMARY KEY, actor_id UUID NOT NULL REFERENCES portal_users(id), target_id UUID NOT NULL REFERENCES portal_users(id),
 action TEXT NOT NULL CHECK(action IN ('application_submit','application_accept','application_reject','document_remove','rename_sub_user')),
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32), version BIGINT NOT NULL CHECK(version>0),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TRIGGER portal_identity_phase5_commands_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_phase5_commands FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TABLE portal_identity_application_history (
 user_id UUID NOT NULL REFERENCES portal_users(id),version BIGINT NOT NULL, snapshot JSONB NOT NULL,
 PRIMARY KEY(user_id,version)
);
CREATE FUNCTION record_identity_application() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
 INSERT INTO portal_identity_application_history VALUES(NEW.user_id,NEW.version,to_jsonb(NEW)); RETURN NEW;
END $$;
CREATE TRIGGER portal_identity_application_history_write AFTER INSERT OR UPDATE ON portal_identity_applications FOR EACH ROW EXECUTE FUNCTION record_identity_application();
CREATE TRIGGER portal_identity_application_history_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON portal_identity_application_history FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER portal_identity_application_retain BEFORE TRUNCATE ON portal_identity_applications FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

CREATE TABLE portal_identity_assets (
 id UUID PRIMARY KEY, actor_id UUID NOT NULL REFERENCES portal_users(id), user_id UUID NOT NULL REFERENCES portal_users(id),
 purpose TEXT NOT NULL CHECK(purpose IN ('profile','application')),
 slot TEXT NOT NULL CHECK(slot IN ('tradeLicense','tinCertificate','travelAgencyLicense','nidCard','logo','attachment')),
 CHECK((purpose='application')=(slot='attachment')),
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32),
 expected_version BIGINT NOT NULL CHECK(expected_version>=0), identity_version BIGINT NOT NULL CHECK(identity_version>0),
 public_id TEXT NOT NULL UNIQUE, format TEXT NOT NULL CHECK(format IN ('pdf','png','jpg','webp','svg')),
 byte_size BIGINT NOT NULL CHECK(byte_size BETWEEN 1 AND 5242880), content_hash TEXT NOT NULL CHECK(content_hash ~ '^[a-f0-9]{64}$'),
 state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared','uploading','ready','unknown','abandoned')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(), completed_at TIMESTAMPTZ,
 CHECK((state='ready')=(completed_at IS NOT NULL)),
 CHECK(public_id='shapon/identity/'||id::text), CHECK(slot<>'logo' OR (format<>'pdf' AND byte_size<=524288)), CHECK(format<>'svg' OR slot='logo')
);
CREATE FUNCTION guard_identity_asset() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
 IF (to_jsonb(NEW)-'state'-'completed_at')<>(to_jsonb(OLD)-'state'-'completed_at') OR NOT (
 (OLD.state='prepared' AND NEW.state IN ('uploading','abandoned')) OR
 (OLD.state='uploading' AND NEW.state IN ('ready','unknown','abandoned')) OR
 (OLD.state='unknown' AND NEW.state IN ('ready','abandoned')))
 THEN RAISE EXCEPTION 'immutable asset or invalid transition'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER portal_identity_asset_guard BEFORE UPDATE ON portal_identity_assets FOR EACH ROW EXECUTE FUNCTION guard_identity_asset();
CREATE TRIGGER portal_identity_assets_retain BEFORE DELETE OR TRUNCATE ON portal_identity_assets FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TABLE portal_identity_document_slots (
 user_id UUID NOT NULL REFERENCES portal_users(id), slot TEXT NOT NULL CHECK(slot IN ('tradeLicense','tinCertificate','travelAgencyLicense','nidCard','logo')),
 asset_id UUID REFERENCES portal_identity_assets(id),version BIGINT NOT NULL DEFAULT 1 CHECK(version>0),
 PRIMARY KEY(user_id,slot)
);
CREATE FUNCTION guard_identity_document_slot() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'retain document removal version'; END IF;
 IF NEW.user_id<>OLD.user_id OR NEW.slot<>OLD.slot THEN RAISE EXCEPTION 'immutable document slot'; END IF;
 NEW.version:=OLD.version+1; RETURN NEW;
END $$;
CREATE TRIGGER portal_identity_document_slot_guard BEFORE UPDATE OR DELETE ON portal_identity_document_slots FOR EACH ROW EXECUTE FUNCTION guard_identity_document_slot();
CREATE TRIGGER portal_identity_document_slots_retain BEFORE TRUNCATE ON portal_identity_document_slots FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

CREATE FUNCTION check_identity_document_owner() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
 IF NEW.asset_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM portal_identity_assets WHERE id=NEW.asset_id AND user_id=NEW.user_id AND slot=NEW.slot AND purpose='profile' AND state='ready') THEN RAISE EXCEPTION 'invalid document owner'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER portal_identity_document_owner BEFORE INSERT OR UPDATE ON portal_identity_document_slots FOR EACH ROW EXECUTE FUNCTION check_identity_document_owner();
CREATE FUNCTION check_identity_application_documents() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
 IF cardinality(NEW.documents)<>(SELECT count(DISTINCT d) FROM unnest(NEW.documents) d) OR EXISTS(SELECT 1 FROM unnest(NEW.documents) d WHERE NOT EXISTS(SELECT 1 FROM portal_identity_assets WHERE id=d AND user_id=NEW.user_id AND purpose='application' AND state='ready')) THEN RAISE EXCEPTION 'invalid application documents'; END IF; RETURN NEW;
END $$;
CREATE TRIGGER portal_identity_application_documents BEFORE INSERT OR UPDATE ON portal_identity_applications FOR EACH ROW EXECUTE FUNCTION check_identity_application_documents();

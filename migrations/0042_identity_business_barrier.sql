-- Identity deletion serializes with new financial/booking/access dependencies.
-- Legacy subjects absent from the canonical registry are unaffected.
CREATE FUNCTION identity_assert_subject_writable(subject TEXT) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
 PERFORM set_config('lock_timeout','2s',true);
 PERFORM pg_advisory_xact_lock(hashtextextended('portal_identity_authority',0));
 IF EXISTS(SELECT 1 FROM portal_users WHERE clerk_user_id=subject AND status IN ('deleting','deleted')) OR EXISTS(SELECT 1 FROM portal_identity_provider_state s WHERE s.subject=identity_assert_subject_writable.subject AND s.deleted) THEN
 RAISE EXCEPTION 'identity terminal; business write denied' USING ERRCODE='23514'; END IF;
END; $$;
CREATE FUNCTION identity_assert_owner_writable(owner UUID) RETURNS void LANGUAGE plpgsql AS $$
DECLARE subject TEXT;
BEGIN
 FOR subject IN SELECT w.owner_key FROM wallet_owners w WHERE w.id=owner AND w.owner_type='user' UNION SELECT u.clerk_user_id FROM wallet_owners w JOIN portal_agencies a ON a.agency_code=w.owner_key JOIN portal_users u ON u.id=a.owner_user_id WHERE w.id=owner AND w.owner_type='agency'
 LOOP PERFORM identity_assert_subject_writable(subject); END LOOP;
END; $$;
CREATE FUNCTION identity_assert_client_writable(client UUID) RETURNS void LANGUAGE plpgsql AS $$
DECLARE subject TEXT; owner UUID;
BEGIN
 FOR subject IN SELECT external_user_id FROM api_clients WHERE id=client UNION SELECT external_user_id FROM portal_staff_clients WHERE client_id=client LOOP PERFORM identity_assert_subject_writable(subject); END LOOP;
 FOR owner IN SELECT owner_id FROM wallet_client_links WHERE client_id=client LOOP PERFORM identity_assert_owner_writable(owner); END LOOP;
END; $$;
CREATE FUNCTION identity_business_barrier() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE j JSONB:=to_jsonb(NEW); subject TEXT; owner UUID; client UUID;
BEGIN
 PERFORM set_config('lock_timeout','2s',true);
 PERFORM pg_advisory_xact_lock(hashtextextended('portal_identity_authority',0));
 -- Revocation and freezing must remain possible during terminal containment.
 IF TG_OP='UPDATE' AND TG_TABLE_NAME='api_clients' AND NOT (j->>'active')::boolean AND NOT (j->>'api_management_enabled')::boolean AND j->>'external_user_id' IS NOT DISTINCT FROM to_jsonb(OLD)->>'external_user_id' THEN RETURN NEW; END IF;
 IF TG_OP='UPDATE' AND TG_TABLE_NAME='client_credentials' AND NOT (j->>'active')::boolean THEN RETURN NEW; END IF;
 IF TG_OP='UPDATE' AND TG_TABLE_NAME='wallet_owners' AND j->>'status'='frozen' THEN RETURN NEW; END IF;
 IF TG_OP='UPDATE' THEN
 FOREACH subject IN ARRAY ARRAY[to_jsonb(OLD)->>'external_user_id',to_jsonb(OLD)->>'created_by_external_user_id',to_jsonb(OLD)->>'creator_external_user_id',to_jsonb(OLD)->>'owner_external_user_id',to_jsonb(OLD)->>'requested_by_user_id',to_jsonb(OLD)->>'actor_id',to_jsonb(OLD)->>'created_by_user_id'] LOOP PERFORM identity_assert_subject_writable(subject); END LOOP;
 IF to_jsonb(OLD) ? 'client_id' THEN PERFORM identity_assert_client_writable((to_jsonb(OLD)->>'client_id')::uuid); END IF;
 END IF;
 FOREACH subject IN ARRAY ARRAY[j->>'external_user_id',j->>'created_by_external_user_id',j->>'creator_external_user_id',j->>'owner_external_user_id',j->>'requested_by_user_id',j->>'reviewed_by_user_id',j->>'actor_id',j->>'created_by_user_id'] LOOP PERFORM identity_assert_subject_writable(subject); END LOOP;
 IF TG_TABLE_NAME='wallet_owners' THEN
 IF NEW.owner_type='user' THEN PERFORM identity_assert_subject_writable(NEW.owner_key); ELSE
 FOR subject IN SELECT u.clerk_user_id FROM portal_agencies a JOIN portal_users u ON u.id=a.owner_user_id WHERE a.agency_code=NEW.owner_key LOOP PERFORM identity_assert_subject_writable(subject); END LOOP; END IF;
 END IF;
 IF j ? 'client_id' THEN PERFORM identity_assert_client_writable((j->>'client_id')::uuid); END IF;
 IF TG_TABLE_NAME='api_clients' AND TG_OP='UPDATE' THEN PERFORM identity_assert_client_writable(NEW.id); END IF;
 IF j ? 'owner_id' THEN PERFORM identity_assert_owner_writable((j->>'owner_id')::uuid); END IF;
 IF j ? 'wallet_account_id' THEN SELECT owner_id INTO owner FROM wallet_accounts WHERE id=(j->>'wallet_account_id')::uuid; PERFORM identity_assert_owner_writable(owner); END IF;
 IF j ? 'booking_id' THEN SELECT client_id,created_by_external_user_id INTO client,subject FROM flight_bookings WHERE id=(j->>'booking_id')::uuid; PERFORM identity_assert_client_writable(client); PERFORM identity_assert_subject_writable(subject); END IF;
 RETURN NEW;
END; $$;
DO $$ DECLARE tab TEXT; BEGIN
 FOREACH tab IN ARRAY ARRAY['api_clients','client_credentials','machine_tokens','portal_prebooking_sessions','portal_staff_clients','wallet_owners','wallet_client_links','wallet_accounts','wallet_operations','wallet_requests','wallet_ledger_entries','portal_hold_drafts','flight_bookings','flight_ticket_issues','flight_cancellations'] LOOP
 EXECUTE format('CREATE TRIGGER identity_business_barrier BEFORE INSERT OR UPDATE ON %I FOR EACH ROW EXECUTE FUNCTION identity_business_barrier()',tab);
 END LOOP;
END; $$;

-- Approved recipient policy: B2B agency/requester only; all four operations
-- roles for ticket-management updates; no archive copies. No historical backfill.
CREATE TABLE portal_notification_deliveries (
 id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 source_key TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('ticket_management','booking','role_changed')),
 channel TEXT NOT NULL CHECK(channel IN ('email','sms')),
 audience TEXT NOT NULL CHECK(audience IN ('customer','internal')),
 user_id UUID NOT NULL REFERENCES portal_users(id),
 agency_id UUID REFERENCES portal_agencies(id),
 recipient TEXT NOT NULL,
 payload JSONB NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','sending','sent','failed','unknown','suppressed')),
 attempts INTEGER NOT NULL DEFAULT 0,
 claim_token UUID,
 claimed_at TIMESTAMPTZ,
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT,
 provider_message_id TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(source_key,channel,recipient,audience)
);
CREATE INDEX portal_notification_pending ON portal_notification_deliveries(created_at,id) WHERE state IN ('pending','failed','sending');
CREATE TABLE portal_notification_attempts (
 token UUID PRIMARY KEY,
 delivery_id UUID NOT NULL REFERENCES portal_notification_deliveries(id),
 outcome TEXT CHECK(outcome IN ('sent','failed','unknown')),
 error_code TEXT,
 provider_message_id TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION protect_portal_notification() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR OLD.state IN ('sent','unknown','suppressed') OR
   (NEW.source_key,NEW.kind,NEW.channel,NEW.audience,NEW.user_id,NEW.agency_id,NEW.recipient,NEW.payload,NEW.created_at)
   IS DISTINCT FROM (OLD.source_key,OLD.kind,OLD.channel,OLD.audience,OLD.user_id,OLD.agency_id,OLD.recipient,OLD.payload,OLD.created_at)
 THEN RAISE EXCEPTION 'notification identity or terminal delivery is immutable'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER notification_immutable BEFORE UPDATE OR DELETE ON portal_notification_deliveries FOR EACH ROW EXECUTE FUNCTION protect_portal_notification();
CREATE TRIGGER notification_no_truncate BEFORE TRUNCATE ON portal_notification_deliveries FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE FUNCTION protect_portal_notification_attempt() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' OR OLD.outcome IS NOT NULL OR (NEW.token,NEW.delivery_id,NEW.created_at) IS DISTINCT FROM (OLD.token,OLD.delivery_id,OLD.created_at)
 THEN RAISE EXCEPTION 'notification attempt is immutable'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER notification_attempt_immutable BEFORE UPDATE OR DELETE ON portal_notification_attempts FOR EACH ROW EXECUTE FUNCTION protect_portal_notification_attempt();
CREATE TRIGGER notification_attempt_no_truncate BEFORE TRUNCATE ON portal_notification_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

CREATE FUNCTION portal_notification_phone(p_user UUID) RETURNS TEXT LANGUAGE plpgsql STABLE AS $$
DECLARE address TEXT;
BEGIN
 SELECT fields->>'agencyMobile' INTO address FROM portal_identity_profiles WHERE user_id=p_user AND kind='profile';
 IF address IS NULL OR address ~ '[^0-9+ ().-]' THEN RETURN ''; END IF;
 address:=regexp_replace(address,'[^0-9]','','g');
 IF left(address,2)='00' THEN address:=substr(address,3); ELSIF left(address,2)='01' THEN address:='88'||address; ELSIF length(address)=10 AND left(address,1)='1' THEN address:='880'||address; END IF;
 RETURN address;
END $$;

CREATE FUNCTION enqueue_portal_notification(p_key TEXT,p_kind TEXT,p_channel TEXT,p_audience TEXT,p_user UUID,p_agency UUID,p_payload JSONB) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE address TEXT;
BEGIN
 IF NOT EXISTS(SELECT 1 FROM portal_users u WHERE u.id=p_user AND u.status='active' AND NOT EXISTS(SELECT 1 FROM portal_identity_provider_state s WHERE s.subject=u.clerk_user_id AND s.deleted)) THEN RETURN; END IF;
 IF p_agency IS NOT NULL AND NOT EXISTS(SELECT 1 FROM portal_agencies a JOIN portal_users u ON u.id=a.owner_user_id WHERE a.id=p_agency AND a.status='active' AND u.status='active') THEN RETURN; END IF;
 IF p_channel='email' THEN SELECT lower(btrim(email)) INTO address FROM portal_users WHERE id=p_user;
 ELSE
  address:=portal_notification_phone(p_user);
 END IF;
 INSERT INTO portal_notification_deliveries(source_key,kind,channel,audience,user_id,agency_id,recipient,payload,state,error_code)
 VALUES(p_key,p_kind,p_channel,p_audience,p_user,p_agency,coalesce(address,''),p_payload,
 CASE WHEN (p_channel='email' AND address ~ '^[^[:space:]@,;<>]+@[^[:space:]@,;<>]+\.[^[:space:]@,;<>]+$' AND length(address)<=320) OR (p_channel='sms' AND address ~ '^[1-9][0-9]{7,14}$') THEN 'pending' ELSE 'suppressed' END,
 CASE WHEN coalesce(address,'')='' THEN 'RECIPIENT_MISSING' ELSE NULL END) ON CONFLICT DO NOTHING;
END $$;

CREATE FUNCTION enqueue_native_ticket_notification() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE r ticket_management_requests; q ticket_management_quotes; u RECORD; owner_id UUID; agency UUID; data JSONB;
BEGIN
 SELECT * INTO STRICT r FROM ticket_management_requests WHERE id=NEW.request_id;
 SELECT * INTO q FROM ticket_management_quotes WHERE id=coalesce(r.approved_quote_id,r.active_quote_id);
 SELECT a.owner_user_id,a.id INTO owner_id,agency FROM wallet_accounts acct JOIN wallet_owners w ON w.id=acct.owner_id JOIN portal_agencies a ON a.agency_code=w.owner_key AND w.owner_type='agency' WHERE acct.id=r.wallet_account_id;
 IF owner_id IS NULL THEN SELECT p.id INTO owner_id FROM wallet_accounts acct JOIN wallet_owners w ON w.id=acct.owner_id JOIN portal_users p ON p.clerk_user_id=w.owner_key AND w.owner_type='user' WHERE acct.id=r.wallet_account_id; END IF;
 data:=jsonb_build_object('action',r.action,'reference',r.public_ref,'status',NEW.to_status,'event',NEW.event_type,'at',NEW.effective_at,'currency',r.currency,'amount',q.data->'customerAmountMinor','deadline',q.confirmation_deadline_at);
 IF NEW.event_type<>'financial-exception' THEN
  FOR u IN SELECT id FROM portal_users WHERE id=owner_id OR (clerk_user_id=r.requested_by_user_id AND (id=owner_id OR EXISTS(SELECT 1 FROM portal_agency_memberships m WHERE m.user_id=portal_users.id AND m.agency_id=agency))) LOOP
   PERFORM enqueue_portal_notification('ticket:'||NEW.id,'ticket_management','email','customer',u.id,agency,data);
  END LOOP;
 END IF;
 FOR u IN SELECT id FROM portal_users WHERE role IN ('staff_support','staff_account','admin','superadmin') LOOP
  PERFORM enqueue_portal_notification('ticket:'||NEW.id,'ticket_management','email','internal',u.id,NULL,data);
 END LOOP;
 RETURN NEW;
END $$;
CREATE TRIGGER native_ticket_notifications AFTER INSERT ON ticket_management_events FOR EACH ROW EXECUTE FUNCTION enqueue_native_ticket_notification();

-- Only canonical portal bookings. Event-time snapshots never look up passenger
-- contacts as recipients and never call suppliers or the reference database.
CREATE FUNCTION enqueue_native_booking_notification(p_booking UUID,p_key TEXT,p_status TEXT) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE b flight_bookings; d portal_hold_drafts; r flight_reprices; u UUID; agency UUID; t JSONB; data JSONB;
BEGIN
 SELECT * INTO b FROM flight_bookings WHERE id=p_booking;
 SELECT * INTO d FROM portal_hold_drafts WHERE id=b.portal_hold_draft_id;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT id INTO u FROM portal_users WHERE clerk_user_id=d.owner_external_user_id;
 IF u IS NULL THEN RETURN; END IF;
 SELECT agency_id INTO agency FROM portal_agency_memberships WHERE user_id=u;
 SELECT * INTO r FROM flight_reprices WHERE id=b.price_id;
 SELECT jsonb_build_object('id',i.id,'state',CASE WHEN v.issue_id IS NOT NULL THEN 'issued' ELSE i.state END,'response',coalesce(v.public_response,i.public_response),'createdAt',i.created_at,'updatedAt',coalesce(v.created_at,i.updated_at),'payment',jsonb_build_object('required',i.wallet_required,'state',coalesce(w.state,'not_attached'),'operationId',w.id)) INTO t FROM flight_ticket_issues i LEFT JOIN flight_ticket_verifications v ON v.issue_id=i.id LEFT JOIN wallet_operations w ON w.subject_kind='ticket_issue' AND w.subject_id=i.id WHERE i.booking_id=b.id;
 data:=jsonb_build_object('status',p_status,'draftId',d.id,'ownerId',d.owner_external_user_id,'owner',d.owner_display,'quote',r.selling->'item1','pricing',r.tier_pricing,'expiresAt',r.expires_at,'accepted',r.accepted_at IS NOT NULL,'submissionEnabled',false,'booking',jsonb_build_object('id',b.id,'reference',b.public_ref,'state',b.state,'response',b.public_response,'ticket',t,'details',jsonb_build_object('createdAt',b.created_at,'ticketingTimeLimit',b.ticketing_time_limit,'passengers',b.request->'passengerInfoes')));
 PERFORM enqueue_portal_notification(p_key,'booking','email','customer',u,agency,data);
 IF p_status='confirmed' THEN PERFORM enqueue_portal_notification(p_key,'booking','sms','customer',u,agency,data); END IF;
END $$;
CREATE FUNCTION enqueue_native_booking_event() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE booking UUID; status TEXT; key TEXT;
BEGIN
 IF TG_TABLE_NAME='flight_bookings' THEN
  IF TG_OP='UPDATE' AND OLD.state=NEW.state THEN RETURN NEW; END IF;
  booking:=NEW.id;status:=CASE NEW.state WHEN 'held' THEN 'on-hold' WHEN 'cancelled' THEN 'cancelled' WHEN 'pending' THEN 'pending' ELSE 'in-progress' END;
  key:='booking:'||NEW.id||':'||NEW.state;
 ELSIF TG_TABLE_NAME='flight_ticket_issues' THEN
  IF TG_OP='UPDATE' AND OLD.state=NEW.state THEN RETURN NEW; END IF;
  booking:=NEW.booking_id;status:=CASE NEW.state WHEN 'issued' THEN 'confirmed' ELSE 'in-progress' END;
  key:='issue:'||NEW.id||':'||status;
 ELSIF TG_TABLE_NAME='flight_cancellations' THEN
  IF TG_OP='UPDATE' AND OLD.state=NEW.state THEN RETURN NEW; END IF;
  booking:=NEW.booking_id;status:=CASE NEW.state WHEN 'cancelled' THEN 'cancelled' ELSE 'in-progress' END;key:='cancel:'||NEW.id||':'||status;
 ELSIF TG_TABLE_NAME='flight_ticket_verifications' THEN
  SELECT booking_id INTO booking FROM flight_ticket_issues WHERE id=NEW.issue_id;
  status:='confirmed';key:='issue:'||NEW.issue_id||':confirmed';
 ELSE RETURN NEW;
 END IF;
 PERFORM enqueue_native_booking_notification(booking,key,status);
 RETURN NEW;
END $$;
CREATE TRIGGER native_booking_notifications AFTER INSERT OR UPDATE OF state ON flight_bookings FOR EACH ROW EXECUTE FUNCTION enqueue_native_booking_event();
CREATE CONSTRAINT TRIGGER native_issue_notifications AFTER INSERT OR UPDATE ON flight_ticket_issues DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION enqueue_native_booking_event();
CREATE CONSTRAINT TRIGGER native_verified_issue_notifications AFTER INSERT ON flight_ticket_verifications DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION enqueue_native_booking_event();

CREATE FUNCTION enqueue_native_role_notification() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF OLD.role<>NEW.role AND NEW.status='active' THEN
  PERFORM enqueue_portal_notification('role:'||NEW.id||':'||NEW.authorization_version,'role_changed','email','customer',NEW.id,NULL,jsonb_build_object('firstName',coalesce(NEW.first_name,''),'previousRole',OLD.role,'nextRole',NEW.role));
 END IF;RETURN NEW;
END $$;
CREATE TRIGGER native_role_notifications AFTER UPDATE OF role ON portal_users FOR EACH ROW EXECUTE FUNCTION enqueue_native_role_notification();
-- Old automatic archive jobs are retained as history and never claimed by the
-- new workers. No existing notification backlog is dispatched by this migration.

CREATE CONSTRAINT TRIGGER native_cancel_notifications AFTER INSERT OR UPDATE ON flight_cancellations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION enqueue_native_booking_event();
UPDATE portal_identity_mail SET state='blocked',error_code='ARCHIVE_COPY_REMOVED' WHERE audience='archive' AND state='pending';

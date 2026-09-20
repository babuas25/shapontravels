-- Opt-in cutover: migration alone never sends messages or replays history.
CREATE TABLE business_notification_dispatch (
 singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
 owner TEXT NOT NULL DEFAULT 'frontend' CHECK(owner IN ('frontend','rust')),
 activated_at TIMESTAMPTZ
);
INSERT INTO business_notification_dispatch(singleton) VALUES(TRUE);

CREATE TABLE business_notification_deliveries (
 id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 source_key TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('booking','ticket_management','deposit')),
 channel TEXT NOT NULL CHECK(channel IN ('email','sms')),
 user_id UUID NOT NULL REFERENCES portal_users(id), agency_id UUID NOT NULL REFERENCES portal_agencies(id),
 recipient TEXT NOT NULL, payload JSONB NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','sending','sent','failed','unknown','suppressed')),
 attempts INTEGER NOT NULL DEFAULT 0, claim_token UUID, claimed_at TIMESTAMPTZ,
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 error_code TEXT, provider_message_id TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(source_key,channel,recipient)
);
CREATE INDEX business_notification_pending ON business_notification_deliveries(next_attempt_at,created_at,id) WHERE state IN ('pending','failed','sending');
CREATE TABLE business_notification_attempts (
 token UUID PRIMARY KEY, delivery_id UUID NOT NULL REFERENCES business_notification_deliveries(id),
 outcome TEXT CHECK(outcome IN ('sent','failed','unknown')),
 error_code TEXT, provider_message_id TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE FUNCTION protect_business_notification() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' OR OLD.state IN ('sent','unknown','suppressed') OR
 (NEW.id,NEW.source_key,NEW.kind,NEW.channel,NEW.user_id,NEW.agency_id,NEW.recipient,NEW.payload,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.source_key,OLD.kind,OLD.channel,OLD.user_id,OLD.agency_id,OLD.recipient,OLD.payload,OLD.created_at)
 THEN RAISE EXCEPTION 'business notification snapshot is immutable'; END IF;
 RETURN NEW; END $$;
CREATE TRIGGER business_notification_immutable BEFORE UPDATE OR DELETE ON business_notification_deliveries FOR EACH ROW EXECUTE FUNCTION protect_business_notification();
CREATE TRIGGER business_notification_no_truncate BEFORE TRUNCATE ON business_notification_deliveries FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER business_attempt_immutable BEFORE UPDATE OR DELETE ON business_notification_attempts FOR EACH ROW EXECUTE FUNCTION protect_portal_notification_attempt();
CREATE TRIGGER business_attempt_no_truncate BEFORE TRUNCATE ON business_notification_attempts FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

CREATE FUNCTION enqueue_business_notification(p_key TEXT,p_kind TEXT,p_channel TEXT,p_user UUID,p_agency UUID,p_payload JSONB) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE address TEXT;
BEGIN
 IF NOT EXISTS(SELECT 1 FROM business_notification_dispatch WHERE owner='rust') THEN RETURN; END IF;
 -- Agency members only. Never passenger contacts, B2C users, staff, or archive copies.
 IF NOT EXISTS(SELECT 1 FROM portal_users u JOIN portal_agency_memberships m ON m.user_id=u.id
   JOIN portal_agencies a ON a.id=m.agency_id JOIN portal_users o ON o.id=a.owner_user_id
   WHERE u.id=p_user AND a.id=p_agency AND u.role IN ('b2b','b2b_sub') AND u.status='active'
   AND a.status='active' AND o.status='active' AND NOT EXISTS(SELECT 1 FROM portal_identity_provider_state s WHERE s.deleted AND s.subject IN (u.clerk_user_id,o.clerk_user_id))) THEN RETURN; END IF;
 IF p_channel='sms' AND NOT coalesce(((p_kind='booking' AND p_payload->>'status'='confirmed') OR
   (p_kind='deposit' AND p_payload->>'event' IN ('deposit_requested','deposit_approved'))),FALSE) THEN RETURN; END IF;
 IF p_channel='email' THEN SELECT lower(btrim(email)) INTO address FROM portal_users WHERE id=p_user;
 ELSE address:=portal_notification_phone(p_user); END IF;
 INSERT INTO business_notification_deliveries(source_key,kind,channel,user_id,agency_id,recipient,payload,state,error_code)
 VALUES(p_key,p_kind,p_channel,p_user,p_agency,coalesce(address,''),p_payload,
 CASE WHEN (p_channel='email' AND address ~ '^[^[:space:]@,;<>]+@[^[:space:]@,;<>]+\.[^[:space:]@,;<>]+$' AND length(address)<=320)
 OR (p_channel='sms' AND address ~ '^8801[3-9][0-9]{8}$') THEN 'pending' ELSE 'suppressed' END,
 CASE WHEN coalesce(address,'')='' THEN 'RECIPIENT_MISSING' ELSE NULL END) ON CONFLICT DO NOTHING;
END $$;

-- Existing transactional event producers keep their durable snapshots. Auth and
-- role email remains on its existing path; business messages select ONE owner.
CREATE OR REPLACE FUNCTION enqueue_portal_notification(p_key TEXT,p_kind TEXT,p_channel TEXT,p_audience TEXT,p_user UUID,p_agency UUID,p_payload JSONB) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE address TEXT;
BEGIN

 IF p_kind IN ('booking','ticket_management') AND EXISTS(SELECT 1 FROM business_notification_dispatch WHERE owner='rust') THEN
  IF p_audience='customer' THEN
   PERFORM enqueue_business_notification(p_key,p_kind,p_channel,p_user,p_agency,
    CASE WHEN p_kind='booking' AND p_payload#>>'{booking,state}'='outcome_unknown' AND p_payload->>'status'='in-progress'
    THEN jsonb_set(p_payload,'{status}','"unconfirmed"') ELSE p_payload END);
  END IF;
  RETURN;
 END IF;
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

CREATE FUNCTION enqueue_business_deposit() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE r wallet_requests; agency UUID; agent UUID; data JSONB;
BEGIN
 IF NOT EXISTS(SELECT 1 FROM business_notification_dispatch WHERE owner='rust') THEN RETURN NEW; END IF;
 UPDATE wallet_notifications SET state='suppressed',error_code='RUST_OWNS_DELIVERY' WHERE id=NEW.id;
 SELECT * INTO STRICT r FROM wallet_requests WHERE id=NEW.request_id;
 SELECT a.id,a.owner_user_id INTO agency,agent FROM wallet_accounts acct JOIN wallet_owners o ON o.id=acct.owner_id
 JOIN portal_agencies a ON o.owner_type='agency' AND a.agency_code=o.owner_key WHERE acct.id=r.wallet_account_id;
 IF agency IS NULL THEN RETURN NEW; END IF;
 data:=jsonb_build_object('requestId',r.id,'event',NEW.event,'reference',r.public_ref,'amount',CASE WHEN NEW.event='deposit_requested' THEN coalesce(r.details->>'gross_amount',r.amount::text) ELSE r.amount::text END,
 'currency',r.currency,'details',r.details,'reviewRemarks',r.review_remarks);
 -- Both request acknowledgement and approval go to the agency owner, never admins.
 PERFORM enqueue_business_notification('deposit:'||NEW.id,'deposit',NEW.channel,agent,agency,data);
 RETURN NEW;
END $$;
CREATE TRIGGER native_business_deposit AFTER INSERT ON wallet_notifications FOR EACH ROW EXECUTE FUNCTION enqueue_business_deposit();

CREATE FUNCTION enqueue_business_import() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE agency UUID; agent UUID; data JSONB;
BEGIN
 IF TG_OP='UPDATE' AND OLD.status=NEW.status THEN RETURN NEW; END IF;
 SELECT a.id,a.owner_user_id INTO agency,agent FROM portal_agencies a WHERE a.agency_code=NEW.agency_code;
 data:=jsonb_build_object('status',NEW.status,'reference',NEW.public_ref,'importId',NEW.id,'itinerary',NEW.data->'itinerary','pnr',NEW.data->'pnr','travellers',NEW.data#>'{passengers,travellers}','ticketNumbers',NEW.data->'ticketNumbers');
 PERFORM enqueue_business_notification('import:'||NEW.id||':'||NEW.status,'booking','email',agent,agency,data);
 IF NEW.status='confirmed' THEN PERFORM enqueue_business_notification('import:'||NEW.id||':confirmed','booking','sms',agent,agency,data); END IF;
 RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER native_business_import AFTER INSERT OR UPDATE ON portal_import_bookings DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION enqueue_business_import();

-- Supplier-confirmed expiry/unconfirmed status, never an inferred cancellation
-- based solely on a local clock or an ambiguous network outcome.
CREATE FUNCTION enqueue_business_pnr_status() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE status TEXT;
BEGIN
 IF NOT NEW.verified THEN RETURN NEW; END IF;
 status:=CASE lower(NEW.response#>>'{item1,status}') WHEN 'expired' THEN 'expired' WHEN 'unconfirmed' THEN 'unconfirmed' WHEN 'cancelled' THEN 'cancelled' ELSE NULL END;
 IF status IS NULL OR EXISTS(SELECT 1 FROM flight_ticket_issues i LEFT JOIN flight_ticket_verifications v ON v.issue_id=i.id WHERE i.booking_id=NEW.booking_id AND (i.state='issued' OR v.issue_id IS NOT NULL)) THEN RETURN NEW; END IF;
 PERFORM enqueue_native_booking_notification(NEW.booking_id,'pnr:'||NEW.booking_id||':'||status,status);
 RETURN NEW;
END $$;
CREATE TRIGGER native_business_pnr AFTER INSERT ON flight_booking_pnr_observations FOR EACH ROW EXECUTE FUNCTION enqueue_business_pnr_status();

-- Native Ticket Management. No legacy database or supplier execution adapter.
-- Requests preserve the existing portal stages; settlement is an Approved outcome.
CREATE SEQUENCE ticket_management_reference_seq MAXVALUE 281474976710655;
CREATE TABLE ticket_management_entitlements (
 id UUID PRIMARY KEY,
 booking_id UUID NOT NULL REFERENCES flight_bookings(id),
 wallet_account_id UUID NOT NULL REFERENCES wallet_accounts(id),
 passenger_index INTEGER NOT NULL CHECK(passenger_index BETWEEN 0 AND 19),
 passenger_name TEXT NOT NULL,
 passenger_type TEXT NOT NULL,
 ticket_number TEXT NOT NULL CHECK(length(ticket_number) BETWEEN 1 AND 80),
 currency TEXT NOT NULL CHECK(currency IN ('BDT','USD')),
 amount BIGINT NOT NULL CHECK(amount BETWEEN 1 AND 9007199254740991),
 -- Each funding slice is backed by a captured operation. Reissue fees never
 -- become refundable entitlement; only its per-passenger fare difference does.
 funding JSONB NOT NULL CHECK(jsonb_typeof(funding)='array'),
 predecessor_id UUID UNIQUE REFERENCES ticket_management_entitlements(id),
 state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active','refunded','reissued','voided')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(booking_id,ticket_number)
);
CREATE UNIQUE INDEX ticket_management_current_passenger ON ticket_management_entitlements(booking_id,passenger_index) WHERE state='active';
CREATE TABLE ticket_management_requests (
 id UUID PRIMARY KEY,
 public_ref TEXT NOT NULL UNIQUE CHECK(public_ref ~ '^TMR[REV][A-F0-9]{12}$'),
 booking_id UUID NOT NULL REFERENCES flight_bookings(id),
 wallet_account_id UUID NOT NULL REFERENCES wallet_accounts(id),
 requested_by_user_id TEXT NOT NULL,
 action TEXT NOT NULL CHECK(action IN ('refund','reissue','void')),
 request_type TEXT NOT NULL CHECK(request_type IN ('voluntary','involuntary')),
 status TEXT NOT NULL DEFAULT 'requested' CHECK(status IN ('requested','in-progress','awaiting-confirmation','approved','rejected','expired')),
 terminal_outcome TEXT,
 version BIGINT NOT NULL DEFAULT 1 CHECK(version BETWEEN 1 AND 9007199254740991),
 currency TEXT NOT NULL CHECK(currency IN ('BDT','USD')),
 request_note TEXT,
 snapshot JSONB NOT NULL CHECK(jsonb_typeof(snapshot)='object'),
 active_quote_id UUID,
 approved_quote_id UUID,
 assignee_user_id TEXT,
 assignee_role TEXT,
 hold_operation_id UUID REFERENCES wallet_operations(id),
 settled_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 status_changed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK((status IN ('requested','in-progress','awaiting-confirmation') AND terminal_outcome IS NULL)
   OR (status='approved' AND (terminal_outcome IS NULL OR terminal_outcome=CASE action WHEN 'refund' THEN 'refunded' WHEN 'reissue' THEN 'reissued' ELSE 'voided' END))
   OR (status='rejected' AND terminal_outcome IS NOT NULL AND terminal_outcome IN ('staff-rejected','customer-rejected'))
   OR (status='expired' AND terminal_outcome IS NOT NULL AND terminal_outcome='confirmation-expired')),
 CHECK((settled_at IS NOT NULL)=(status='approved' AND terminal_outcome IS NOT NULL))
);
CREATE UNIQUE INDEX ticket_management_one_open_action ON ticket_management_requests(booking_id,action)
 WHERE status IN ('requested','in-progress','awaiting-confirmation') OR (status='approved' AND terminal_outcome IS NULL);
CREATE INDEX ticket_management_queue ON ticket_management_requests(created_at DESC,id DESC);
CREATE TABLE ticket_management_selections (
 request_id UUID NOT NULL REFERENCES ticket_management_requests(id),
 entitlement_id UUID NOT NULL REFERENCES ticket_management_entitlements(id),
 PRIMARY KEY(request_id,entitlement_id)
);
-- Removing a claim only frees an unsettled ticket for another request. Its
-- permanent selection, request and events remain immutable evidence.
CREATE TABLE ticket_management_claims (
 entitlement_id UUID PRIMARY KEY REFERENCES ticket_management_entitlements(id),
 request_id UUID NOT NULL REFERENCES ticket_management_requests(id)
);
CREATE TABLE ticket_management_quotes (
 id UUID PRIMARY KEY,
 request_id UUID NOT NULL REFERENCES ticket_management_requests(id),
 quote_version INTEGER NOT NULL CHECK(quote_version>0),
 data JSONB NOT NULL CHECK(jsonb_typeof(data)='object'),
 confirmation_deadline_at TIMESTAMPTZ NOT NULL,
 published_by_user_id TEXT NOT NULL,
 published_by_role TEXT NOT NULL,
 published_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(request_id,quote_version),
 UNIQUE(request_id,id)
);
ALTER TABLE ticket_management_requests ADD CONSTRAINT tm_active_quote_owner
 FOREIGN KEY(id,active_quote_id) REFERENCES ticket_management_quotes(request_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE ticket_management_requests ADD CONSTRAINT tm_approved_quote_owner
 FOREIGN KEY(id,approved_quote_id) REFERENCES ticket_management_quotes(request_id,id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE ticket_management_events (
 id UUID PRIMARY KEY,
 request_id UUID NOT NULL REFERENCES ticket_management_requests(id),
 request_version BIGINT NOT NULL,
 event_type TEXT NOT NULL,
 actor_id TEXT NOT NULL,
 actor_role TEXT NOT NULL,
 from_status TEXT,
 to_status TEXT NOT NULL,
 note TEXT,
 metadata JSONB NOT NULL DEFAULT '{}',
 effective_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(request_id,request_version)
);
CREATE TABLE ticket_management_replays (
 actor_id TEXT NOT NULL,
 request_key UUID NOT NULL,
 request_hash BYTEA NOT NULL,
 request_id UUID NOT NULL REFERENCES ticket_management_requests(id),
 result JSONB NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(actor_id,request_key)
);
CREATE TABLE ticket_management_outbox (
 event_id UUID PRIMARY KEY REFERENCES ticket_management_events(id),
 request_id UUID NOT NULL REFERENCES ticket_management_requests(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE FUNCTION protect_ticket_management_entitlement() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' THEN RAISE EXCEPTION 'ticket entitlement is immutable'; END IF;
 IF OLD.state<>'active' OR NEW.state='active'
   OR (to_jsonb(NEW)-'state') IS DISTINCT FROM (to_jsonb(OLD)-'state') THEN
   RAISE EXCEPTION 'ticket entitlement identity or terminal state is immutable';
 END IF;
 RETURN NEW;
END; $$;
-- Read projection for the native booking receipt. Original supplier issuance
-- evidence stays immutable; subsequent manual settlements supply current tickets.
CREATE VIEW ticket_management_receipts AS
 SELECT b.id booking_id,jsonb_build_object(
   'management',COALESCE((SELECT jsonb_agg(jsonb_build_object('passengerIndex',e.passenger_index,'ticketNumber',e.ticket_number,'state',e.state) ORDER BY e.passenger_index)
     FROM ticket_management_entitlements e WHERE e.booking_id=b.id AND e.state<>'reissued'),'[]'::jsonb),
   'requestReferences',COALESCE((SELECT jsonb_agg(jsonb_build_object('action',m.action,'publicRef',m.public_ref,'status',m.status,'terminalOutcome',m.terminal_outcome) ORDER BY m.created_at,m.id)
     FROM ticket_management_requests m WHERE m.booking_id=b.id),'[]'::jsonb),
   'managementPaymentState',(SELECT CASE WHEN COALESCE(sum(p.refunded_amount),0)=0 THEN 'captured'
      WHEN sum(p.refunded_amount)=sum(p.amount) THEN 'refunded' ELSE 'partially-refunded' END
     FROM wallet_operations p WHERE p.booking_id=b.id AND p.state='captured')
 ) metadata FROM flight_bookings b;

-- Financial completion and approval cannot be saved without the matching
-- immutable quote, decision event, ticket states and exact kernel posting.
CREATE FUNCTION ticket_management_integrity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE r ticket_management_requests; q ticket_management_quotes; op wallet_operations;
 amount BIGINT; direction TEXT; actual NUMERIC; selected INTEGER; claimed INTEGER;
BEGIN
 SELECT * INTO r FROM ticket_management_requests WHERE id=NEW.id;
 SELECT count(*) INTO selected FROM ticket_management_selections WHERE request_id=r.id;
 SELECT count(*) INTO claimed FROM ticket_management_claims WHERE request_id=r.id;
 IF selected<1 OR selected>20 OR NOT EXISTS(SELECT 1 FROM ticket_management_events WHERE request_id=r.id AND request_version=r.version) THEN
   RAISE EXCEPTION 'ticket request requires selections and an immutable event';
 END IF;
 IF (r.terminal_outcome IS NULL AND claimed<>selected) OR (r.terminal_outcome IS NOT NULL AND claimed<>0) THEN
   RAISE EXCEPTION 'ticket request claims disagree';
 END IF;
 IF r.status='awaiting-confirmation' AND r.active_quote_id IS NULL THEN RAISE EXCEPTION 'ticket quotation required'; END IF;
 IF r.status='approved' THEN
   SELECT * INTO q FROM ticket_management_quotes WHERE id=r.approved_quote_id AND request_id=r.id;
   IF NOT FOUND OR r.active_quote_id IS DISTINCT FROM q.id OR NOT EXISTS(
      SELECT 1 FROM ticket_management_events WHERE request_id=r.id AND event_type='customer-approved' AND metadata->>'quoteId'=q.id::text
   ) THEN RAISE EXCEPTION 'ticket customer approval required'; END IF;
   amount:=(q.data->>'customerAmountMinor')::bigint; direction:=q.data->>'direction';
   IF direction='debit' THEN
     SELECT * INTO op FROM wallet_operations WHERE id=r.hold_operation_id;
     IF NOT FOUND OR op.wallet_account_id<>r.wallet_account_id OR op.booking_id<>r.booking_id OR op.subject_kind<>'ticket_management'
        OR op.subject_id<>q.id OR op.amount<>amount OR op.currency<>r.currency
        OR op.state<>(CASE WHEN r.terminal_outcome IS NULL THEN 'reserved' ELSE 'captured' END) THEN
       RAISE EXCEPTION 'ticket payment differs from approved quotation';
     END IF;
   ELSIF r.hold_operation_id IS NOT NULL THEN RAISE EXCEPTION 'unexpected ticket debit hold';
   END IF;
   IF r.terminal_outcome IS NOT NULL THEN
     IF EXISTS(SELECT 1 FROM ticket_management_selections s JOIN ticket_management_entitlements e ON e.id=s.entitlement_id WHERE s.request_id=r.id AND e.state<>r.terminal_outcome) THEN
       RAISE EXCEPTION 'settled ticket entitlement mismatch';
     END IF;
     SELECT COALESCE(sum(l.amount),0) INTO actual FROM wallet_ledger_entries l
       WHERE l.metadata->>'ticketManagementRequestId'=r.id::text AND l.transaction_type='refund' AND l.wallet_account_id=r.wallet_account_id AND l.booking_id=r.booking_id;
     IF actual<>(CASE WHEN direction='credit' THEN amount ELSE 0 END) THEN
       RAISE EXCEPTION 'ticket credit differs from approved quotation';
     END IF;
     IF r.action='reissue' AND EXISTS(
       SELECT 1 FROM ticket_management_selections s JOIN ticket_management_entitlements e ON e.id=s.entitlement_id
       LEFT JOIN ticket_management_entitlements successor ON successor.predecessor_id=e.id
       WHERE s.request_id=r.id AND (successor.id IS NULL OR successor.amount<>e.amount+COALESCE((
         SELECT (a->>'fareDifferenceAmountMinor')::bigint FROM jsonb_array_elements(q.data->'reissueFareDifferenceAllocations') a WHERE a->>'entitlementId'=e.id::text
       ),-1))
     ) THEN RAISE EXCEPTION 'reissue successor differs from accepted allocation'; END IF;
   END IF;
 END IF;
 RETURN NULL;
END; $$;
CREATE CONSTRAINT TRIGGER ticket_management_integrity AFTER INSERT OR UPDATE ON ticket_management_requests
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ticket_management_integrity();
CREATE TRIGGER ticket_management_entitlement_immutable BEFORE UPDATE OR DELETE ON ticket_management_entitlements
 FOR EACH ROW EXECUTE FUNCTION protect_ticket_management_entitlement();
CREATE FUNCTION protect_ticket_management_request() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP<>'UPDATE' THEN RAISE EXCEPTION 'ticket request cannot be removed'; END IF;
 IF OLD.terminal_outcome IS NOT NULL OR NEW.version<>OLD.version+1
   OR (NEW.id,NEW.public_ref,NEW.booking_id,NEW.wallet_account_id,NEW.requested_by_user_id,NEW.action,NEW.request_type,NEW.currency,NEW.request_note,NEW.snapshot,NEW.created_at)
      IS DISTINCT FROM
      (OLD.id,OLD.public_ref,OLD.booking_id,OLD.wallet_account_id,OLD.requested_by_user_id,OLD.action,OLD.request_type,OLD.currency,OLD.request_note,OLD.snapshot,OLD.created_at) THEN
   RAISE EXCEPTION 'ticket request identity or terminal result is immutable';
 END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER ticket_management_request_immutable BEFORE UPDATE OR DELETE ON ticket_management_requests
 FOR EACH ROW EXECUTE FUNCTION protect_ticket_management_request();
DO $$ DECLARE tab TEXT; BEGIN
 FOREACH tab IN ARRAY ARRAY['ticket_management_selections','ticket_management_quotes','ticket_management_events','ticket_management_replays','ticket_management_outbox'] LOOP
  EXECUTE format('CREATE TRIGGER tm_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON %I FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation()',tab);
 END LOOP;
 FOREACH tab IN ARRAY ARRAY['ticket_management_requests','ticket_management_entitlements'] LOOP
  EXECUTE format('CREATE TRIGGER tm_no_truncate BEFORE TRUNCATE ON %I FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation()',tab);
  EXECUTE format('CREATE TRIGGER identity_business_barrier BEFORE INSERT OR UPDATE ON %I FOR EACH ROW EXECUTE FUNCTION identity_business_barrier()',tab);
 END LOOP;
 FOREACH tab IN ARRAY ARRAY['ticket_management_events','ticket_management_replays'] LOOP
  EXECUTE format('CREATE TRIGGER identity_business_barrier BEFORE INSERT ON %I FOR EACH ROW EXECUTE FUNCTION identity_business_barrier()',tab);
 END LOOP;
END; $$;

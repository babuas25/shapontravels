-- One financial booking identity for native and imported tickets. Original
-- supplier evidence, captured operations and ledger history remain unchanged.
CREATE TABLE booking_financial_subjects (
 id UUID PRIMARY KEY,
 native_booking_id UUID UNIQUE REFERENCES flight_bookings(id),
 imported_booking_id UUID UNIQUE REFERENCES portal_import_bookings(id),
 CHECK ((native_booking_id IS NOT NULL)::int + (imported_booking_id IS NOT NULL)::int = 1),
 CHECK (id=coalesce(native_booking_id,imported_booking_id))
);
INSERT INTO booking_financial_subjects(id,native_booking_id) SELECT id,id FROM flight_bookings;
INSERT INTO booking_financial_subjects(id,imported_booking_id) SELECT id,id FROM portal_import_bookings;
CREATE FUNCTION register_booking_financial_subject() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_TABLE_NAME='flight_bookings' THEN
  INSERT INTO booking_financial_subjects(id,native_booking_id) VALUES(NEW.id,NEW.id);
 ELSE
  INSERT INTO booking_financial_subjects(id,imported_booking_id) VALUES(NEW.id,NEW.id);
 END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER register_financial_booking AFTER INSERT ON flight_bookings FOR EACH ROW EXECUTE FUNCTION register_booking_financial_subject();
CREATE TRIGGER register_financial_import AFTER INSERT ON portal_import_bookings FOR EACH ROW EXECUTE FUNCTION register_booking_financial_subject();
CREATE TRIGGER financial_subject_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON booking_financial_subjects FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
ALTER TABLE ticket_management_entitlements DROP CONSTRAINT ticket_management_entitlements_booking_id_fkey,
 ADD FOREIGN KEY(booking_id) REFERENCES booking_financial_subjects(id);
ALTER TABLE ticket_management_requests DROP CONSTRAINT ticket_management_requests_booking_id_fkey,
 ADD FOREIGN KEY(booking_id) REFERENCES booking_financial_subjects(id);
ALTER TABLE wallet_operations DROP CONSTRAINT wallet_operations_booking_id_fkey,
 ADD FOREIGN KEY(booking_id) REFERENCES booking_financial_subjects(id);
ALTER TABLE wallet_ledger_entries DROP CONSTRAINT wallet_ledger_entries_booking_id_fkey,
 ADD FOREIGN KEY(booking_id) REFERENCES booking_financial_subjects(id);

CREATE OR REPLACE VIEW ticket_management_receipts AS
 SELECT b.id booking_id,jsonb_build_object(
   'management',COALESCE((SELECT jsonb_agg(jsonb_build_object('passengerIndex',e.passenger_index,'ticketNumber',e.ticket_number,'state',e.state) ORDER BY e.passenger_index)
     FROM ticket_management_entitlements e WHERE e.booking_id=b.id AND e.state<>'reissued'),'[]'::jsonb),
   'requestReferences',COALESCE((SELECT jsonb_agg(jsonb_build_object('action',m.action,'publicRef',m.public_ref,'status',m.status,'terminalOutcome',m.terminal_outcome) ORDER BY m.created_at,m.id)
     FROM ticket_management_requests m WHERE m.booking_id=b.id),'[]'::jsonb),
   'managementPaymentState',(SELECT CASE WHEN COALESCE(sum(p.refunded_amount),0)=0 THEN 'captured'
      WHEN sum(p.refunded_amount)=sum(p.amount) THEN 'refunded' ELSE 'partially-refunded' END
     FROM wallet_operations p WHERE (p.booking_id=b.id OR p.id=(SELECT operation_id FROM portal_import_bookings WHERE id=b.imported_booking_id)) AND p.state='captured')
 ) metadata FROM booking_financial_subjects b;

CREATE OR REPLACE FUNCTION wallet_apply_posting(
 p_id UUID,p_account UUID,p_type TEXT,p_amount BIGINT,p_operation UUID,p_booking UUID,
 p_key TEXT,p_hash BYTEA,p_actor TEXT,p_role TEXT,p_remarks TEXT,p_metadata JSONB
) RETURNS wallet_ledger_entries
LANGUAGE plpgsql SECURITY DEFINER SET search_path=public,pg_temp AS $$
DECLARE
 a wallet_accounts; w wallet_owners; old wallet_ledger_entries; result wallet_ledger_entries; operation wallet_operations;
 da BIGINT:=0; dh BIGINT:=0; next_available BIGINT; next_hold BIGINT;
BEGIN
 -- Match application transactions: authority barrier before any wallet rows.
 -- The ledger/account triggers reacquire this transaction-level lock.
 PERFORM set_config('lock_timeout','2s',true);
 PERFORM pg_advisory_xact_lock(hashtextextended('portal_identity_authority',0));
 SELECT o.* INTO STRICT w FROM wallet_owners o JOIN wallet_accounts acct ON acct.owner_id=o.id WHERE acct.id=p_account FOR UPDATE OF o;
 SELECT * INTO STRICT a FROM wallet_accounts WHERE id=p_account FOR UPDATE;
 SELECT * INTO old FROM wallet_ledger_entries WHERE wallet_account_id=p_account AND idempotency_key=p_key;
 IF FOUND THEN
   IF old.request_hash<>p_hash OR old.transaction_type<>p_type OR old.amount<>p_amount
      OR old.operation_id IS DISTINCT FROM p_operation OR old.booking_id IS DISTINCT FROM p_booking THEN
     RAISE EXCEPTION 'IDEMPOTENCY_KEY_REUSED' USING ERRCODE='P0001';
   END IF;
   RETURN old;
 END IF;
 IF p_amount<=0 OR p_actor='' OR p_role='' THEN RAISE EXCEPTION 'INVALID_WALLET_POSTING' USING ERRCODE='P0001'; END IF;
 IF p_type IN ('booking_hold','booking_confirm','hold_release','refund') THEN
   SELECT * INTO operation FROM wallet_operations WHERE id=p_operation FOR UPDATE;
   IF NOT FOUND OR operation.wallet_account_id<>p_account OR operation.currency<>a.currency
      OR (operation.booking_id IS DISTINCT FROM p_booking AND NOT (
        p_type='refund' AND operation.booking_id IS NULL AND operation.subject_kind='manual_issue'
        AND EXISTS(SELECT 1 FROM portal_import_bookings b WHERE b.id=p_booking
          AND b.id=operation.subject_id AND b.operation_id=operation.id AND b.status='confirmed')
      )) THEN
     RAISE EXCEPTION 'WALLET_RESERVATION_MISMATCH' USING ERRCODE='P0001';
   END IF;
   IF p_type='refund' THEN
     IF operation.state<>'captured' OR p_amount>operation.amount-operation.refunded_amount THEN
       RAISE EXCEPTION 'REFUND_EXCEEDS_CAPTURE' USING ERRCODE='P0001';
     END IF;
   ELSIF operation.amount<>p_amount OR operation.state NOT IN ('reserved','reconciliation') THEN
     RAISE EXCEPTION 'WALLET_RESERVATION_MISMATCH' USING ERRCODE='P0001';
   END IF;
 END IF;
 IF p_type IN ('booking_hold','manual_debit') AND w.status<>'active' THEN RAISE EXCEPTION 'WALLET_FROZEN' USING ERRCODE='P0001'; END IF;
 CASE p_type
  WHEN 'deposit','manual_credit','refund' THEN da:=p_amount;
  WHEN 'manual_debit' THEN da:=-p_amount;
  WHEN 'booking_hold' THEN da:=-p_amount; dh:=p_amount;
  WHEN 'booking_confirm' THEN dh:=-p_amount;
  WHEN 'hold_release' THEN da:=p_amount; dh:=-p_amount;
  ELSE RAISE EXCEPTION 'INVALID_WALLET_POSTING' USING ERRCODE='P0001';
 END CASE;
 next_available:=a.available_balance+da; next_hold:=a.hold_balance+dh;
 IF next_available<0 THEN RAISE EXCEPTION 'INSUFFICIENT_FUNDS' USING ERRCODE='P0001'; END IF;
 IF next_hold<0 THEN RAISE EXCEPTION 'WALLET_RESERVATION_MISMATCH' USING ERRCODE='P0001'; END IF;
 INSERT INTO wallet_ledger_entries(id,wallet_account_id,sequence,transaction_type,amount,currency,
  available_before,available_after,hold_before,hold_after,operation_id,booking_id,booking_reference,
  idempotency_key,request_hash,created_by_user_id,created_by_role,remarks,metadata)
 VALUES(p_id,p_account,a.version+1,p_type,p_amount,a.currency,a.available_balance,next_available,a.hold_balance,next_hold,
  p_operation,p_booking,coalesce((SELECT public_ref FROM flight_bookings WHERE id=p_booking),(SELECT coalesce(booking_reference,public_ref) FROM portal_import_bookings WHERE id=p_booking)),p_key,p_hash,p_actor,p_role,p_remarks,p_metadata)
 RETURNING * INTO result;
 UPDATE wallet_accounts SET available_balance=next_available,hold_balance=next_hold,version=a.version+1,updated_at=result.created_at WHERE id=p_account;
 IF p_type IN ('booking_confirm','hold_release') THEN
   UPDATE wallet_operations SET state=CASE WHEN p_type='booking_confirm' THEN 'captured' ELSE 'released' END,updated_at=result.created_at WHERE id=p_operation;
 ELSIF p_type='refund' THEN
   UPDATE wallet_operations SET refunded_amount=refunded_amount+p_amount,updated_at=result.created_at WHERE id=p_operation;
 END IF;
 INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata)
 VALUES('system',p_actor,'wallet.'||p_type,'wallet_account',p_account::text,jsonb_build_object('entryId',p_id,'actorRole',p_role));
 RETURN result;
END;
$$;

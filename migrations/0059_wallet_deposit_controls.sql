-- Preserve historical requests/postings, including any pre-existing duplicates.
-- Enforce external-payment uniqueness on NEW approvals instead of adding a
-- unique index that could fail deployment or rewriting financial history.
CREATE FUNCTION wallet_deposit_payment_identity(d JSONB) RETURNS TEXT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE method TEXT:=d->>'method'; destination TEXT; reference TEXT;
BEGIN
 reference:=NULLIF(lower(btrim(d->>'reference_number')),'');
 IF reference IS NULL THEN RETURN NULL; END IF;
 IF method='mobile' THEN
   destination:=COALESCE(NULLIF(lower(btrim(d#>>'{mfs_account,accountNumber}')),''),d->>'mfs_account_id');
   RETURN jsonb_build_array('mobile',lower(btrim(d->>'mfs_provider')),destination,reference)::text;
 ELSIF method IN ('bank','bank_transfer') THEN
   destination:=COALESCE(NULLIF(lower(btrim(d#>>'{company_bank_account,accountNumber}')),''),d->>'company_bank_account_id');
   -- Bank references can recur on different deposit dates. Bank and transfer
   -- are the same incoming payment namespace; sender/owner/amount are not keys.
   RETURN jsonb_build_array('bank',destination,d->>'deposit_date',reference)::text;
 ELSIF method='cheque' THEN
   destination:=COALESCE(NULLIF(lower(btrim(d#>>'{company_bank_account,accountNumber}')),''),d->>'company_bank_account_id');
   RETURN jsonb_build_array('cheque',destination,lower(btrim(d->>'cheque_issued_bank')),d->>'cheque_issued_date',reference)::text;
 END IF;
 -- Cash has no externally verifiable transaction identifier.
 RETURN NULL;
END; $$;
CREATE INDEX wallet_approved_payment_identity ON wallet_requests(wallet_deposit_payment_identity(details))
 WHERE kind='deposit' AND status='approved';

-- A unique claim also prevents write skew under stronger transaction isolation.
-- Pick one historical claimant per payment without touching duplicate credits.
CREATE TABLE wallet_deposit_payment_claims (
 payment_identity TEXT PRIMARY KEY,
 request_id UUID NOT NULL REFERENCES wallet_requests(id) DEFERRABLE INITIALLY DEFERRED,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
INSERT INTO wallet_deposit_payment_claims(payment_identity,request_id)
 SELECT DISTINCT ON (wallet_deposit_payment_identity(details)) wallet_deposit_payment_identity(details),id
 FROM wallet_requests WHERE kind='deposit' AND status='approved' AND wallet_deposit_payment_identity(details) IS NOT NULL
 ORDER BY wallet_deposit_payment_identity(details),requested_at,id;
CREATE TRIGGER wallet_payment_claim_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON wallet_deposit_payment_claims
 FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

ALTER TABLE wallet_requests ADD COLUMN reversal_of UUID REFERENCES wallet_requests(id);
CREATE INDEX wallet_deposit_reversals ON wallet_requests(reversal_of) WHERE reversal_of IS NOT NULL;
-- Reports now include post-ticket operations; avoid a full ledger scan per booking.
CREATE INDEX wallet_operations_booking_payments ON wallet_operations(booking_id,wallet_account_id,currency)
 WHERE subject_kind IN ('ticket_issue','ticket_management');

CREATE FUNCTION wallet_request_financial_controls() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=public,pg_temp AS $$
DECLARE original wallet_requests; reversed NUMERIC; identity TEXT; claimant UUID;
BEGIN
 PERFORM set_config('lock_timeout','2s',true);
 PERFORM pg_advisory_xact_lock(hashtextextended('portal_identity_authority',0));
 IF TG_OP='UPDATE' AND NEW.reversal_of IS DISTINCT FROM OLD.reversal_of THEN
   RAISE EXCEPTION 'deposit reversal binding is immutable';
 END IF;
 IF NEW.kind='deposit' AND NEW.status='approved' THEN
   identity:=wallet_deposit_payment_identity(NEW.details);
   IF identity IS NOT NULL THEN
     INSERT INTO wallet_deposit_payment_claims(payment_identity,request_id) VALUES(identity,NEW.id)
       ON CONFLICT(payment_identity) DO NOTHING;
     SELECT request_id INTO claimant FROM wallet_deposit_payment_claims WHERE payment_identity=identity;
     IF claimant IS DISTINCT FROM NEW.id THEN
       RAISE EXCEPTION 'DEPOSIT_PAYMENT_ALREADY_CREDITED' USING ERRCODE='P0001';
     END IF;
   END IF;
 END IF;
 IF NEW.reversal_of IS NOT NULL THEN
   SELECT * INTO original FROM wallet_requests WHERE id=NEW.reversal_of;
   IF NOT FOUND OR original.kind<>'deposit' OR original.status<>'approved'
     OR original.ledger_entry_id IS NULL OR NEW.kind<>'adjustment'
     OR NEW.details->>'adjustment_type' IS DISTINCT FROM 'debit'
     OR (NEW.wallet_account_id,NEW.currency) IS DISTINCT FROM (original.wallet_account_id,original.currency) THEN
     RAISE EXCEPTION 'INVALID_DEPOSIT_REVERSAL' USING ERRCODE='P0001';
   END IF;
   IF NEW.status<>'rejected' THEN
     SELECT COALESCE(sum(amount),0) INTO reversed FROM wallet_requests
       WHERE reversal_of=original.id AND status='approved' AND id<>NEW.id;
     IF NEW.amount::numeric>original.amount::numeric-reversed THEN
       RAISE EXCEPTION 'DEPOSIT_REVERSAL_EXCEEDS_CREDIT' USING ERRCODE='P0001';
     END IF;
   END IF;
 END IF;
 RETURN NEW;
END; $$;
CREATE TRIGGER wallet_request_financial_controls BEFORE INSERT OR UPDATE ON wallet_requests
 FOR EACH ROW EXECUTE FUNCTION wallet_request_financial_controls();

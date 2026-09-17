-- Fresh financial records only. Existing booking and identity data are retained.
CREATE TABLE wallet_owners (
    id UUID PRIMARY KEY,
    owner_type TEXT NOT NULL CHECK (owner_type IN ('agency','user')),
    owner_key TEXT NOT NULL CHECK (length(owner_key) BETWEEN 1 AND 128),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','frozen')),
    display JSONB NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(display)='object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(owner_type,owner_key)
);
CREATE TABLE wallet_client_links (
    client_id UUID PRIMARY KEY REFERENCES api_clients(id),
    owner_id UUID NOT NULL REFERENCES wallet_owners(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE wallet_accounts (
    id UUID PRIMARY KEY,
    owner_id UUID NOT NULL REFERENCES wallet_owners(id),
    currency TEXT NOT NULL CHECK (currency IN ('BDT','USD')),
    scale SMALLINT NOT NULL DEFAULT 2 CHECK (scale=2),
    available_balance BIGINT NOT NULL DEFAULT 0 CHECK (available_balance>=0),
    hold_balance BIGINT NOT NULL DEFAULT 0 CHECK (hold_balance>=0),
    version BIGINT NOT NULL DEFAULT 0 CHECK (version>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(owner_id,currency),
    CHECK (available_balance::numeric+hold_balance::numeric<=9223372036854775807)
);
CREATE TABLE wallet_operations (
    id UUID PRIMARY KEY,
    wallet_account_id UUID NOT NULL REFERENCES wallet_accounts(id),
    subject_kind TEXT NOT NULL CHECK (subject_kind IN ('ticket_issue','ticket_management','manual_issue')),
    subject_id UUID NOT NULL,
    booking_id UUID REFERENCES flight_bookings(id),
    amount BIGINT NOT NULL CHECK (amount>0),
    currency TEXT NOT NULL CHECK (currency IN ('BDT','USD')),
    state TEXT NOT NULL CHECK (state IN ('reserved','reconciliation','captured','released')),
    refunded_amount BIGINT NOT NULL DEFAULT 0 CHECK (refunded_amount>=0 AND refunded_amount<=amount),
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash)=32),
    actor_id TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(subject_kind,subject_id)
);
CREATE TABLE wallet_ledger_entries (
    id UUID PRIMARY KEY,
    wallet_account_id UUID NOT NULL REFERENCES wallet_accounts(id),
    sequence BIGINT NOT NULL CHECK (sequence>0),
    transaction_type TEXT NOT NULL CHECK (transaction_type IN ('deposit','booking_hold','booking_confirm','hold_release','refund','manual_credit','manual_debit')),
    amount BIGINT NOT NULL CHECK (amount>0),
    currency TEXT NOT NULL CHECK (currency IN ('BDT','USD')),
    available_before BIGINT NOT NULL CHECK (available_before>=0),
    available_after BIGINT NOT NULL CHECK (available_after>=0),
    hold_before BIGINT NOT NULL CHECK (hold_before>=0),
    hold_after BIGINT NOT NULL CHECK (hold_after>=0),
    operation_id UUID REFERENCES wallet_operations(id),
    booking_id UUID REFERENCES flight_bookings(id),
    booking_reference TEXT,
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 200),
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash)=32),
    created_by_user_id TEXT NOT NULL,
    created_by_role TEXT NOT NULL,
    remarks TEXT NOT NULL DEFAULT '' CHECK (length(remarks)<=1000),
    metadata JSONB NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(metadata)='object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(wallet_account_id,sequence),
    UNIQUE(wallet_account_id,idempotency_key)
);
CREATE INDEX wallet_ledger_time ON wallet_ledger_entries(wallet_account_id,created_at,sequence);
CREATE UNIQUE INDEX wallet_once_per_reservation ON wallet_ledger_entries(operation_id,transaction_type)
    WHERE operation_id IS NOT NULL AND transaction_type IN ('booking_hold','booking_confirm','hold_release');
CREATE TRIGGER wallet_ledger_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON wallet_ledger_entries
    FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE TRIGGER wallet_links_immutable BEFORE UPDATE OR DELETE OR TRUNCATE ON wallet_client_links
    FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

CREATE FUNCTION protect_wallet_identity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_TABLE_NAME='wallet_owners' THEN
   IF
    (NEW.id,NEW.owner_type,NEW.owner_key,NEW.created_at) IS DISTINCT FROM
    (OLD.id,OLD.owner_type,OLD.owner_key,OLD.created_at) THEN
    RAISE EXCEPTION 'wallet owner identity is immutable';
   END IF;
 ELSIF TG_TABLE_NAME='wallet_accounts' THEN
   IF
    (NEW.id,NEW.owner_id,NEW.currency,NEW.scale) IS DISTINCT FROM
    (OLD.id,OLD.owner_id,OLD.currency,OLD.scale) THEN
    RAISE EXCEPTION 'wallet account identity is immutable';
   END IF;
 ELSIF TG_TABLE_NAME='wallet_operations' THEN
    IF (NEW.id,NEW.wallet_account_id,NEW.subject_kind,NEW.subject_id,NEW.booking_id,NEW.amount,NEW.currency,NEW.request_hash,NEW.actor_id,NEW.actor_role,NEW.created_at)
       IS DISTINCT FROM
       (OLD.id,OLD.wallet_account_id,OLD.subject_kind,OLD.subject_id,OLD.booking_id,OLD.amount,OLD.currency,OLD.request_hash,OLD.actor_id,OLD.actor_role,OLD.created_at)
       OR (OLD.state IN ('captured','released') AND NEW.state<>OLD.state)
       OR NEW.refunded_amount<OLD.refunded_amount THEN
      RAISE EXCEPTION 'wallet operation identity or terminal state is immutable';
    END IF;
 END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER wallet_owner_identity BEFORE UPDATE ON wallet_owners FOR EACH ROW EXECUTE FUNCTION protect_wallet_identity();
CREATE TRIGGER wallet_account_identity BEFORE UPDATE ON wallet_accounts FOR EACH ROW EXECUTE FUNCTION protect_wallet_identity();
CREATE TRIGGER wallet_operation_identity BEFORE UPDATE ON wallet_operations FOR EACH ROW EXECUTE FUNCTION protect_wallet_identity();

CREATE FUNCTION wallet_account_integrity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE a wallet_accounts; last_entry wallet_ledger_entries; reserved numeric;
BEGIN
 SELECT * INTO a FROM wallet_accounts WHERE id=NEW.id;
 SELECT * INTO last_entry FROM wallet_ledger_entries WHERE wallet_account_id=a.id ORDER BY sequence DESC LIMIT 1;
 IF FOUND THEN
   IF (a.version,a.available_balance,a.hold_balance) IS DISTINCT FROM (last_entry.sequence,last_entry.available_after,last_entry.hold_after) THEN
     RAISE EXCEPTION 'wallet ledger and balance disagree';
   END IF;
 ELSIF (a.version,a.available_balance,a.hold_balance)<>(0,0,0) THEN
   RAISE EXCEPTION 'fresh wallet must start at zero';
 END IF;
 SELECT COALESCE(sum(amount),0) INTO reserved FROM wallet_operations WHERE wallet_account_id=a.id AND state IN ('reserved','reconciliation');
 IF a.hold_balance::numeric<>reserved THEN RAISE EXCEPTION 'wallet holds and reservations disagree'; END IF;
 RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER wallet_account_integrity AFTER INSERT OR UPDATE ON wallet_accounts
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION wallet_account_integrity();

CREATE FUNCTION wallet_operation_integrity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE o wallet_operations; held BIGINT; captured BIGINT; released BIGINT; refunded numeric;
BEGIN
 SELECT * INTO o FROM wallet_operations WHERE id=NEW.id;
 SELECT count(*) FILTER(WHERE transaction_type='booking_hold'),count(*) FILTER(WHERE transaction_type='booking_confirm'),
        count(*) FILTER(WHERE transaction_type='hold_release'),COALESCE(sum(amount) FILTER(WHERE transaction_type='refund'),0)
 INTO held,captured,released,refunded FROM wallet_ledger_entries WHERE operation_id=o.id;
 IF held<>1 OR (o.state IN ('reserved','reconciliation') AND captured+released<>0)
    OR (o.state='captured' AND (captured<>1 OR released<>0))
    OR (o.state='released' AND (released<>1 OR captured<>0)) OR o.refunded_amount::numeric<>refunded THEN
   RAISE EXCEPTION 'wallet reservation and ledger disagree';
 END IF;
 RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER wallet_operation_integrity AFTER INSERT OR UPDATE ON wallet_operations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION wallet_operation_integrity();

-- The only runtime path that posts balance changes. The caller authorizes the
-- business operation, while this primitive enforces exact amounts and replay.
CREATE FUNCTION wallet_apply_posting(
 p_id UUID,p_account UUID,p_type TEXT,p_amount BIGINT,p_operation UUID,p_booking UUID,
 p_key TEXT,p_hash BYTEA,p_actor TEXT,p_role TEXT,p_remarks TEXT,p_metadata JSONB
) RETURNS wallet_ledger_entries
LANGUAGE plpgsql SECURITY DEFINER SET search_path=public,pg_temp AS $$
DECLARE
 a wallet_accounts; w wallet_owners; old wallet_ledger_entries; result wallet_ledger_entries; operation wallet_operations;
 da BIGINT:=0; dh BIGINT:=0; next_available BIGINT; next_hold BIGINT;
BEGIN
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
      OR operation.booking_id IS DISTINCT FROM p_booking THEN
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
  p_operation,p_booking,(SELECT public_ref FROM flight_bookings WHERE id=p_booking),p_key,p_hash,p_actor,p_role,p_remarks,p_metadata)
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
REVOKE ALL ON FUNCTION wallet_apply_posting(UUID,UUID,TEXT,BIGINT,UUID,UUID,TEXT,BYTEA,TEXT,TEXT,TEXT,JSONB) FROM PUBLIC;

ALTER TABLE api_clients DROP CONSTRAINT api_clients_permissions_check;
ALTER TABLE api_clients ADD CONSTRAINT api_clients_permissions_check CHECK
 (permissions <@ ARRAY['search:read','booking','cancellation','ticketing','wallet:read']::TEXT[]);

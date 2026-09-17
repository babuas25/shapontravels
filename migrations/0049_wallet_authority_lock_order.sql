-- Preserve amounts, replay, runtime grants and integrity checks. Only lock order changes.
-- Direct calls to this SECURITY DEFINER primitive also need the identity barrier
-- before owner/account locks; otherwise its ledger trigger can deadlock with reserve.
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

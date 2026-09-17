CREATE TABLE wallet_settings (
 id UUID PRIMARY KEY,
 kind TEXT NOT NULL CHECK (kind IN ('bank','mfs','sender','branch')),
 owner_id UUID REFERENCES wallet_owners(id),
 data JSONB NOT NULL CHECK (jsonb_typeof(data)='object'),
 active BOOLEAN NOT NULL DEFAULT TRUE,
 version BIGINT NOT NULL DEFAULT 1 CHECK(version>0),
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK ((kind='sender')=(owner_id IS NOT NULL))
);
CREATE INDEX wallet_settings_owner ON wallet_settings(owner_id,kind,active);
CREATE UNIQUE INDEX wallet_setting_bank_number ON wallet_settings((data->>'accountNumber')) WHERE kind='bank';
CREATE UNIQUE INDEX wallet_setting_mfs_channel ON wallet_settings(lower(data->>'mfsName'),(data->>'paymentType')) WHERE kind='mfs';
CREATE UNIQUE INDEX wallet_setting_branch_name ON wallet_settings(lower(data->>'name')) WHERE kind='branch';
CREATE UNIQUE INDEX wallet_setting_sender_number ON wallet_settings(owner_id,(data->>'accountNumber')) WHERE kind='sender' AND active;
-- Application default branch, without any balances or payment account data.
INSERT INTO wallet_settings(id,kind,data) VALUES
 ('00000000-0000-4000-8000-000000000001','branch','{"name":"Head Office","address":"Shomobai Shopping Market (2nd Floor), Dhankhola Bazar, Gangni, Meherpur-7110"}');

CREATE TABLE wallet_reference_counters (
 kind TEXT NOT NULL, day DATE NOT NULL, last_value INTEGER NOT NULL CHECK(last_value BETWEEN 1 AND 999999),
 PRIMARY KEY(kind,day)
);
CREATE TABLE wallet_requests (
 id UUID PRIMARY KEY,
 kind TEXT NOT NULL CHECK(kind IN ('deposit','adjustment','settlement')),
 public_ref TEXT NOT NULL UNIQUE,
 wallet_account_id UUID NOT NULL REFERENCES wallet_accounts(id),
 amount BIGINT NOT NULL CHECK(amount>0),
 currency TEXT NOT NULL CHECK(currency IN ('BDT','USD')),
 details JSONB NOT NULL CHECK(jsonb_typeof(details)='object'),
 request_hash BYTEA NOT NULL CHECK(octet_length(request_hash)=32),
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','rejected')),
 requested_by_user_id TEXT NOT NULL,
 requested_by_role TEXT NOT NULL,
 reviewed_by_user_id TEXT,
 review_remarks TEXT CHECK(length(review_remarks)<=1000),
 ledger_entry_id UUID UNIQUE REFERENCES wallet_ledger_entries(id),
 requested_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 reviewed_at TIMESTAMPTZ,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 CHECK ((status='pending' AND reviewed_by_user_id IS NULL AND reviewed_at IS NULL)
     OR (status<>'pending' AND reviewed_by_user_id IS NOT NULL AND reviewed_at IS NOT NULL)),
 CHECK (reviewed_by_user_id IS NULL OR reviewed_by_user_id<>requested_by_user_id),
 CHECK (status='approved' OR ledger_entry_id IS NULL)
);
CREATE INDEX wallet_requests_account ON wallet_requests(wallet_account_id,kind,requested_at,id);
CREATE FUNCTION protect_wallet_request() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.kind,NEW.public_ref,NEW.wallet_account_id,NEW.amount,NEW.currency,NEW.details,NEW.request_hash,NEW.requested_by_user_id,NEW.requested_by_role,NEW.requested_at)
 IS DISTINCT FROM
 (OLD.id,OLD.kind,OLD.public_ref,OLD.wallet_account_id,OLD.amount,OLD.currency,OLD.details,OLD.request_hash,OLD.requested_by_user_id,OLD.requested_by_role,OLD.requested_at)
 OR OLD.status<>'pending' THEN RAISE EXCEPTION 'wallet request is immutable'; END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER wallet_request_immutable BEFORE UPDATE ON wallet_requests FOR EACH ROW EXECUTE FUNCTION protect_wallet_request();
CREATE TRIGGER wallet_request_no_delete BEFORE DELETE OR TRUNCATE ON wallet_requests FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();

CREATE TABLE wallet_notifications (
 id UUID PRIMARY KEY,
 request_id UUID NOT NULL REFERENCES wallet_requests(id),
 event TEXT NOT NULL CHECK(event IN ('deposit_requested','deposit_approved','deposit_rejected')),
 channel TEXT NOT NULL CHECK(channel IN ('email','sms')),
 event_key TEXT NOT NULL UNIQUE,
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','sending','sent','failed','unknown')),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0),
 claim_token UUID,
 claimed_at TIMESTAMPTZ,
 completed_at TIMESTAMPTZ,
 error_code TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX wallet_notification_queue ON wallet_notifications(state,created_at);

-- Request decisions are append-only financial outcomes. The actual credited
-- amount/account must agree with their ledger entry at transaction commit.
CREATE FUNCTION wallet_request_integrity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE r wallet_requests; entry wallet_ledger_entries;
BEGIN
 SELECT * INTO r FROM wallet_requests WHERE id=NEW.id;
 IF r.status='approved' THEN
   SELECT * INTO entry FROM wallet_ledger_entries WHERE id=r.ledger_entry_id;
   IF NOT FOUND OR entry.wallet_account_id<>r.wallet_account_id OR entry.currency<>r.currency OR entry.amount<>r.amount THEN
     RAISE EXCEPTION 'approved wallet request requires a matching ledger entry';
   END IF;
   IF entry.idempotency_key<>('request:' || r.id::text)
     OR (r.kind='deposit' AND entry.transaction_type<>'deposit')
     OR (r.kind='adjustment' AND entry.transaction_type IS DISTINCT FROM
       CASE r.details->>'adjustment_type' WHEN 'credit' THEN 'manual_credit' WHEN 'debit' THEN 'manual_debit' END) THEN
     RAISE EXCEPTION 'approved wallet request posting type mismatch';
   END IF;
 END IF;
 RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER wallet_request_integrity AFTER INSERT OR UPDATE ON wallet_requests
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION wallet_request_integrity();

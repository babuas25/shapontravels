-- Native import records have immutable owner and monetary snapshots. They never
-- masquerade as supplier-created flight_bookings or reuse their mutation keys.
CREATE TABLE portal_import_quotes (
 id UUID PRIMARY KEY, actor_id UUID NOT NULL REFERENCES portal_users(id),
 assigned_user_id UUID NOT NULL REFERENCES portal_users(id), agency_code TEXT NOT NULL REFERENCES portal_agencies(agency_code),
 source TEXT NOT NULL CHECK(source IN ('MANUAL','IMP_EXP','SUPPLIER_API')),
 provider TEXT NOT NULL, supplier_reference TEXT NOT NULL,
 currency TEXT NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 gross_minor BIGINT NOT NULL CHECK(gross_minor>=0), payable_minor BIGINT NOT NULL CHECK(payable_minor>0), supplier_minor BIGINT NOT NULL CHECK(supplier_minor>=0),
 data JSONB NOT NULL, fingerprint BYTEA NOT NULL,
 authorized BOOLEAN NOT NULL DEFAULT FALSE,
 expires_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()+INTERVAL '10 minutes',
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE portal_import_bookings (
 id UUID PRIMARY KEY, public_ref TEXT NOT NULL UNIQUE,
 quote_id UUID NOT NULL REFERENCES portal_import_quotes(id),
 creator_id UUID NOT NULL REFERENCES portal_users(id), assigned_user_id UUID NOT NULL REFERENCES portal_users(id),
 agency_code TEXT NOT NULL REFERENCES portal_agencies(agency_code),
 source TEXT NOT NULL CHECK(source IN ('MANUAL','IMP_EXP','SUPPLIER_API')),provider TEXT NOT NULL,supplier_reference TEXT NOT NULL,
 currency TEXT NOT NULL,gross_minor BIGINT NOT NULL CHECK(gross_minor>=0),payable_minor BIGINT NOT NULL CHECK(payable_minor>0),supplier_minor BIGINT NOT NULL CHECK(supplier_minor>=0),
 status TEXT NOT NULL CHECK(status IN ('on-hold','confirmed','cancelled','expired')),
 data JSONB NOT NULL, operation_id UUID REFERENCES wallet_operations(id),
 issued_at TIMESTAMPTZ, version BIGINT NOT NULL DEFAULT 1,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(), updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(provider,supplier_reference), CHECK((status='confirmed')=(issued_at IS NOT NULL)), CHECK(status<>'confirmed' OR operation_id IS NOT NULL)
);
CREATE TABLE portal_import_requests (
 actor_id UUID NOT NULL REFERENCES portal_users(id), request_id UUID NOT NULL, fingerprint BYTEA NOT NULL,
 booking_id UUID NOT NULL REFERENCES portal_import_bookings(id), PRIMARY KEY(actor_id,request_id)
);
CREATE FUNCTION protect_portal_import() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF TG_OP<>'UPDATE' THEN RAISE EXCEPTION 'Import records cannot be deleted'; END IF;
 IF (NEW.id,NEW.public_ref,NEW.quote_id,NEW.creator_id,NEW.assigned_user_id,NEW.agency_code,NEW.source,NEW.provider,NEW.supplier_reference,NEW.currency,NEW.gross_minor,NEW.payable_minor,NEW.supplier_minor,NEW.created_at)
 IS DISTINCT FROM (OLD.id,OLD.public_ref,OLD.quote_id,OLD.creator_id,OLD.assigned_user_id,OLD.agency_code,OLD.source,OLD.provider,OLD.supplier_reference,OLD.currency,OLD.gross_minor,OLD.payable_minor,OLD.supplier_minor,OLD.created_at)
 OR NEW.version<>OLD.version+1 OR (OLD.operation_id IS NOT NULL AND NEW.operation_id IS DISTINCT FROM OLD.operation_id)
 OR OLD.status='confirmed' THEN RAISE EXCEPTION 'Import ownership, pricing and issued records are immutable'; END IF;
 RETURN NEW; END $$;
CREATE TRIGGER portal_import_guard BEFORE UPDATE OR DELETE ON portal_import_bookings FOR EACH ROW EXECUTE FUNCTION protect_portal_import();
CREATE TRIGGER portal_import_no_truncate BEFORE TRUNCATE ON portal_import_bookings FOR EACH STATEMENT EXECUTE FUNCTION reject_audit_mutation();
CREATE INDEX portal_import_agency ON portal_import_bookings(agency_code,issued_at DESC,id);

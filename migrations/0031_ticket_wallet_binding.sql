-- Existing supplier evidence is retained without inventing historical charges.
ALTER TABLE flight_ticket_issues ADD COLUMN wallet_required BOOLEAN NOT NULL DEFAULT FALSE;
CREATE FUNCTION protect_ticket_wallet_flag() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.wallet_required IS DISTINCT FROM OLD.wallet_required THEN
   RAISE EXCEPTION 'ticket financial policy is immutable';
 END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER ticket_wallet_flag BEFORE UPDATE ON flight_ticket_issues
 FOR EACH ROW EXECUTE FUNCTION protect_ticket_wallet_flag();

CREATE FUNCTION ticket_wallet_integrity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE t flight_ticket_issues; op wallet_operations; account wallet_accounts;
        owner UUID; snapshot JSONB;
BEGIN
 SELECT * INTO t FROM flight_ticket_issues WHERE id=NEW.id;
 IF NOT t.wallet_required THEN RETURN NULL; END IF;
 SELECT * INTO op FROM wallet_operations WHERE subject_kind='ticket_issue' AND subject_id=t.id;
 IF NOT FOUND OR op.booking_id IS DISTINCT FROM t.booking_id THEN
   RAISE EXCEPTION 'ticket dispatch requires a wallet reservation';
 END IF;
 SELECT * INTO account FROM wallet_accounts WHERE id=op.wallet_account_id;
 SELECT owner_id INTO owner FROM wallet_client_links WHERE client_id=t.client_id;
 SELECT r.tier_pricing INTO snapshot FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id WHERE b.id=t.booking_id;
 IF owner IS NULL OR account.owner_id<>owner OR account.currency<>op.currency
    OR snapshot IS NULL OR snapshot->>'currency' IS DISTINCT FROM op.currency
    OR (snapshot->>'payable')::numeric*100 IS DISTINCT FROM op.amount::numeric THEN
   RAISE EXCEPTION 'ticket wallet does not match accepted owner and payable';
 END IF;
 RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER ticket_wallet_integrity AFTER INSERT OR UPDATE ON flight_ticket_issues
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION ticket_wallet_integrity();

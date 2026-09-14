-- Existing clients start at Basic; historical quotes/bookings are not repriced.
ALTER TABLE api_clients ADD COLUMN tier TEXT NOT NULL DEFAULT 'basic'
    CHECK (tier IN ('basic','professional','enterprise'));
ALTER TABLE flight_offers ADD COLUMN tier_pricing JSONB;
ALTER TABLE flight_reprices ADD COLUMN tier_pricing JSONB;

CREATE FUNCTION preserve_tier_pricing() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tier_pricing IS DISTINCT FROM OLD.tier_pricing THEN
        RAISE EXCEPTION 'tier pricing snapshots are immutable';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER immutable_offer_tier_pricing BEFORE UPDATE ON flight_offers
FOR EACH ROW EXECUTE FUNCTION preserve_tier_pricing();
CREATE TRIGGER immutable_reprice_tier_pricing BEFORE UPDATE ON flight_reprices
FOR EACH ROW EXECUTE FUNCTION preserve_tier_pricing();

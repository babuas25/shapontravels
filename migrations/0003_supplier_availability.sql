-- Incremented on disable so re-enabling cannot revive stale unbooked offers.
ALTER TABLE supplier_connections ADD COLUMN availability_epoch BIGINT NOT NULL DEFAULT 1 CHECK (availability_epoch > 0);

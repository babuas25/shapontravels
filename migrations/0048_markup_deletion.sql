-- Live rules may be removed without deleting immutable pricing/audit evidence.
ALTER TABLE markup_rules ADD COLUMN archived_at TIMESTAMPTZ;
ALTER TABLE markup_rules ADD CONSTRAINT archived_markup_inactive
    CHECK (archived_at IS NULL OR NOT active);

-- Versions are self-contained immutable snapshots. Offers/reprices continue to
-- reference them, including an in-flight search that retained a deleted rule.
-- Do not cascade, delete, or relax the immutable-version trigger.
ALTER TABLE markup_rule_versions DROP CONSTRAINT markup_rule_versions_rule_id_fkey;
CREATE INDEX flight_offers_markup_rule_idx ON flight_offers(rule_id);
CREATE INDEX flight_reprices_markup_rule_idx ON flight_reprices(rule_id);

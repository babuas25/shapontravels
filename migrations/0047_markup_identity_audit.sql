-- Keep legacy audit authors and immutable rule history; new dashboard changes
-- reference the canonical user without creating a legacy administrator account.
ALTER TABLE markup_rule_versions ALTER COLUMN changed_by DROP NOT NULL;
ALTER TABLE markup_rule_versions ADD COLUMN changed_by_user_id UUID REFERENCES portal_users(id);
ALTER TABLE markup_rule_versions ADD CONSTRAINT markup_version_one_author
    CHECK ((changed_by IS NULL) <> (changed_by_user_id IS NULL));

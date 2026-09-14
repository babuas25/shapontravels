-- Shares are percentages of the resolved markup, configurable without a release.
CREATE TABLE b2b_tier_policy (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    basic INTEGER NOT NULL DEFAULT 60,
    professional INTEGER NOT NULL DEFAULT 80,
    enterprise INTEGER NOT NULL DEFAULT 100,
    CHECK (0 <= basic AND basic <= professional AND professional <= enterprise AND enterprise <= 100)
);
INSERT INTO b2b_tier_policy(singleton) VALUES(TRUE);

CREATE TABLE flight_reprices (
 id UUID PRIMARY KEY,
 offer_id UUID NOT NULL,
 client_id UUID NOT NULL REFERENCES api_clients(id),
 version BIGINT NOT NULL CHECK (version > 0),
 original JSONB NOT NULL,
 selling JSONB NOT NULL,
 reference_map JSONB NOT NULL,
 rule_id UUID NOT NULL,
 rule_version BIGINT NOT NULL,
 audience TEXT NOT NULL,
 agent_id UUID,
 currency TEXT NOT NULL,
 accepted_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL,
 FOREIGN KEY(offer_id,client_id) REFERENCES flight_offers(id,client_id),
 FOREIGN KEY(rule_id,rule_version) REFERENCES markup_rule_versions(rule_id,version),
 UNIQUE(offer_id,version)
);
CREATE INDEX flight_reprices_offer_idx ON flight_reprices(offer_id,version DESC);

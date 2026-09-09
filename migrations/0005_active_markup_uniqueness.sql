-- User-approved policy: exactly one active rule per audience/agent/airline/route.
-- NULL means all airlines/all routes; NULLS NOT DISTINCT prevents bypassing scope
-- uniqueness with nullable fields. Currency is not a separate duplicate scope.
CREATE UNIQUE INDEX markup_one_active_scope ON markup_rules
(audience,agent_id,airline,origin,destination) NULLS NOT DISTINCT WHERE active;

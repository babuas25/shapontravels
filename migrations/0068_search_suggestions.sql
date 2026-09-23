-- Successful searches create private per-actor history.  Unlinked machine
-- clients use a stable client scope; there is intentionally no user foreign
-- key because this table retains historical evidence across subject remaps.
CREATE TABLE flight_search_history (
    id UUID PRIMARY KEY,
    actor_key TEXT NOT NULL CHECK (char_length(actor_key) BETWEEN 1 AND 256),
    search_key CHAR(64) NOT NULL CHECK (search_key ~ '^[a-f0-9]{64}$'),
    popularity_key CHAR(64) NOT NULL CHECK (popularity_key ~ '^[a-f0-9]{64}$'),
    trip_type TEXT NOT NULL CHECK (trip_type IN ('oneway','round','multicity')),
    routes JSONB NOT NULL,
    first_departure_date DATE NOT NULL,
    adults SMALLINT NOT NULL CHECK (adults BETWEEN 1 AND 9),
    children SMALLINT NOT NULL CHECK (children BETWEEN 0 AND 8),
    infants SMALLINT NOT NULL CHECK (infants BETWEEN 0 AND 9),
    children_ages JSONB NOT NULL,
    cabin_class SMALLINT NOT NULL CHECK (cabin_class BETWEEN 1 AND 5),
    preferred_carriers JSONB NOT NULL,
    searched_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(actor_key, search_key),
    CHECK (adults + children + infants <= 9),
    CHECK (infants <= adults),
    CHECK (jsonb_typeof(routes) = 'array'),
    CHECK (jsonb_array_length(routes) BETWEEN 1 AND 6),
    CHECK (jsonb_typeof(children_ages) = 'array'),
    CHECK (jsonb_typeof(preferred_carriers) = 'array')
);
CREATE INDEX flight_search_history_recent_idx
    ON flight_search_history(actor_key, searched_at DESC);
CREATE INDEX flight_search_history_popular_idx
    ON flight_search_history(first_departure_date, searched_at DESC, popularity_key);

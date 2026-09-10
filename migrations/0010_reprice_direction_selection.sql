-- Historical revisions predate explicit selection; never infer/backfill their choice.
ALTER TABLE flight_reprices ADD COLUMN selected_directions JSONB
 CHECK (selected_directions IS NULL OR jsonb_typeof(selected_directions) = 'object');

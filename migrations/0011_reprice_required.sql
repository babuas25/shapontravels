-- A rejected revalidation must not leave a previously accepted quote usable.
ALTER TABLE flight_offers ADD COLUMN reprice_required BOOLEAN NOT NULL DEFAULT FALSE;

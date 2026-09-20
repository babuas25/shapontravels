-- pg_restore clears search_path. The generated booking_reference column invokes
-- this SQL function while restoring its table, so its helper must be qualified.
-- Keep migration 0056 unchanged for installations that already applied it.
CREATE OR REPLACE FUNCTION public.import_booking_reference(data JSONB)
RETURNS TEXT LANGUAGE SQL IMMUTABLE AS $$
 SELECT public.booking_reference_from_evidence(
  jsonb_build_object(
   'item2', jsonb_build_object('isSuccess', true),
   'item1', jsonb_build_object('pnr', data->'pnr', 'airlinesPNR', data->'airlinesPnr')
  ),
  NULL,
  false
 )
$$;

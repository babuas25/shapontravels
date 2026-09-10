# Search cleanup — 2026-09-10

The user authorized adding cleanup and then pushing the combined memory/cleanup change. That authorization applies to this release; the general instruction to obtain permission before future commits/pushes remains in force.

## Policy and implementation

- Search validity stays at 10 minutes. Payload cleanup eligibility starts at offer age 15 minutes, only after both offer and parent Search expire.
- Any RePrice or booking reference protects the offer. All related business records and necessary Search headers are retained. No cascade deletion or cleanup of selected business records occurs.
- Every 30 seconds the serving process drains short batches. At most 512 offer rows, 512 empty Search headers and 512 old ID markers are removed per transaction. It checks a five-second budget between batches and caps a tick at 32 batches. Each statement has a two-second timeout and lock waits are limited to 200 ms. Backlog, load and locks can delay deletion beyond 15 minutes; this is an eligibility threshold, not an exact deletion deadline.
- FOR UPDATE SKIP LOCKED avoids rows being used by RePrice/acceptance/booking. An advisory transaction lock coordinates cleanup instances. Foreign keys remain the final protection against concurrent dependent writes.
- Original/selling/reference-map payloads are deleted, with only owner-scoped offer ID and deletion time stored in `expired_flight_offers` for 24 hours. The marker and payload deletion share a transaction. FareRules/RePrice return owner 410, foreign client 404; after marker expiry they return 404.
- Parent Search payloads are removed only when empty and expired/old enough. Booked or repriced offers retain their parent even if other offers in that Search are removed.
- Migration `0012_search_cleanup.sql` creates the ID-marker table and lookup/cleanup indexes. No existing table is truncated and no business row is cascaded.
- Worker starts only in serving mode after schema and HTTP binding checks. It stops on server shutdown; migration/bootstrap/offline router replays do not run it. Logs contain only policy/count/timing values or safe generic failure text.

## Validation

Local formatting, all-target clippy, all 39 unit/foundation/fixture tests and PostgreSQL 18 integration passed on the combined memory/cleanup code (integration including the real scheduler completed in 37 seconds). Cleanup integration checks:

1. 1,025 newly seeded old offers plus existing unused fixtures are drained in bounded batches; first batch removes exactly 512.
2. A locked old offer survives concurrent cleanup and is removed after releasing its lock. Empty old Search headers disappear only after all their offers are gone.
3. Expired 12-minute offers, old still-valid offers and expired offers under a still-valid parent are retained.
4. Existing RePrice/booking records and their selected offers survive, even when aged beyond the cleanup threshold.
5. A deliberate marker-insert failure rolls back payload deletion. Advisory-lock contention causes a harmless skipped batch.
6. Both public FareRules and RePrice return owner 410 and foreign-client 404 after deletion. Marker expiry/removal results in 404.
7. Repeated cleanup becomes idle. The actual 30-second worker path removes an eligible Search, and shutdown exits promptly (including before the first tick).

The prior memory tests additionally prove original/selling fidelity, exact decimal lexemes and rollback after 130 attempted inserts when final summary validation fails. The measured 35%/42% replay RSS reduction is recorded in [memory evidence](SEARCH_MEMORY_LIFETIMES_2026-09-10.md); the background worker is intentionally disabled in those replay examples.

## Operational limits

Linked RePrice/booking data remains under a separate future retention policy. The 24-hour ID marker contains no offer payload. SQL deletion makes storage available for PostgreSQL reuse/vacuum; immediate disk-file shrinkage is not claimed. No supplier calls or booking/issue mutations are needed to verify cleanup. Production capacity under concurrent Search plus a cleanup backlog remains unmeasured.

Cleanup applies to the active database; backup retention is a separate existing policy.

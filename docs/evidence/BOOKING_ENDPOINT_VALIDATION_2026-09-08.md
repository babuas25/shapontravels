# Hold Booking implementation validation — 2026-09-08

Implemented public Book, owner-only local booking status, and read-only PNR reconciliation. Contract and limitations: [BOOKING_API.md](../BOOKING_API.md).

## Verified locally

- Concurrent same-key requests reserve and dispatch exactly one mock Book; retries replay the stored result, while payload changes and a second key for the same offer are rejected.
- Latest accepted RePrice, client permission, commercial authorization, hold-only intent, passenger age/type and refreshed-reference requirements are enforced before dispatch.
- Synthetic successful hold persists PNR, deadline, original and public responses; accepted selling total remains BDT 4,533.05, with no second markup. Reused supplier reference strings are rebound to the correct platform identity by field.
- Timeout, supplier business failure, unexpected ticket evidence and changed-price responses all remain `outcome_unknown`; repeated requests never make a second mock mutation.
- Owned status returns saved response; foreign-client status returns 404.
- Reconciliation with known PNR/references uses only a mock PNR read. No references after timeout/business failure yields manual-reconciliation-required. PNR evidence does not automatically clear pricing/direct-issue discrepancies.
- A localhost HTTP transport test proves Book does not retry 401 or 5xx. Environment-disabled transport sends zero Book calls.
- Disposable PostgreSQL migration/integration suite passed (`booking_endpoint_final_test`), including previous auth, markup, Search and RePrice regression checks.

Ordinary Rust suite and Clippy are checked separately. No supplier network request was made during this implementation: all booking/PNR results above are synthetic mocks or localhost HTTP responses, not real reservations. No migration or restart of the user's running application database/server occurred.

## Remaining before production

Production supplier adapters deny commercial authorization by default. Payment/credit policy and its per-client/per-quote integration, production passenger-data protection/retention, direct-issue policy, manual reconciliation resolution and ticket lifecycle are not completed. The endpoint is not claimed production booking-ready, and no supplier Book/Cancel/NewTicket was authorized or executed in this work.

## User-approved intentional repeats — follow-up

Migration 0009 supersedes the unique-offer restriction above. A new client-scoped idempotency key represents a separate user intent, including identical passengers and quote; the platform no longer rejects it because of an earlier booking. Same-key replay and payload-conflict protection remain. Bookings also no longer block new RePrice/acceptance.

Updated tests passed for sequential and concurrent different-key bookings with distinct platform booking IDs, independent old-key replay, and a new intentional key alongside an unresolved prior intent. Same-key uncertain replay still dispatches exactly once. Disposable PostgreSQL suite (`repeat_booking_final_test`), ordinary tests and all-target Clippy passed. No live supplier booking or running-database migration was performed.

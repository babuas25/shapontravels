# Held-ticket issue without a PNR call — 15 September 2026

## Request and reviewed contract

The user requested review of the whole ticketing path, then explicitly requested removing the PNR call from **Search → RePrice → Book → NewTicket** because PNR lookup may fail independently of ticketing. The platform's explicit local price acceptance remains between RePrice and Book.

Reviewed the supplied *2_Flight_API_Documentation.pdf*, v2.0 dated 7 May 2026: the pipeline on page 3, Book response and direct-ticket branch on pages 21–24, NewTicket on pages 26–28, PNR on pages 33–35, and errors on pages 36–37. Also reviewed the Rust issue/Book/supplier adapters, cancellation, ticket recovery/report verification, database reservation triggers, portal Hold capabilities, UAT example and existing tests/evidence.

## Findings and decisions

1. **PNR lookup is not required to build NewTicket.** Its six fields come from the saved Book response and original Search/RePrice references. The existing `supplier_payload` already constructs that request without PNR data. The mandatory lookup was a local preflight policy. Its transport, business-response, status and deadline failures could stop issue before NewTicket was sent.
2. **Removing only the network call would leave another blocker.** The old reservation required verified PNR evidence under 30 seconds old and a parseable `lastTicketTime`. Those requirements have also been removed. The original verified Hold and local state are checked under the reservation lock instead.
3. **Deadlines cannot be assumed complete or UTC.** The PDF Book example has an empty `ticketingTimeLimit`. Retained portal UAT evidence records Book as `15/09/2026 12:30:00` and PNR as `09/15/2026 12:30:00`, with no supplier timezone. NewTicket checks explicit-offset RFC3339 deadlines locally, retaining the dispatch-timeout-plus-30-second margin. Missing, empty, unsupported and offset-free values are recorded as requiring supplier validation. No timezone is invented. The supplier must validate the live hold and deadline; the backend does not claim that local evidence proves either remains current.
4. **Previously known contradictory evidence is retained.** An optional, already verified PNR observation still vetoes cancelled/ticketed status, ticket numbers or contradictory references. `Booked` and the observed `Created` held-status variant are accepted. A verified observation's deadline supersedes Book's deadline, including when omitted. Failed/unverified observations are not authoritative and do not replace a previously verified observation. The second review added immutable history to enforce this distinction. None of this reads PNR over the network.
5. **Duplicate and cancellation protections remain essential.** Client/booking locks, unique issue reservation, Issue/Cancel exclusion triggers, key/payload checks, owner/permission checks, accepted held fare and supplier/environment controls remain. A timeout, authentication/transport failure, invalid receipt or supplier business failure after reservation remains `outcome_unknown`; replay cannot send another issue.
6. **Preflight JSON remains compatible; evidence retention now requires a migration.** New preflight JSON identifies `source=saved_booking` and records Book evidence, any verified saved PNR observation, deadline/source/check and timestamp. Historical preflight JSON remains immutable. The second review introduced migration 0028 to preserve PNR history independently of the latest-attempt fields. This supersedes the initial no-migration statement. Apply 0028 before serving the reviewed build. Older evidence already overwritten before upgrade cannot be reconstructed.
7. **The example must follow the same boundary.** The opt-in UAT example now saves the actual issue preflight and does not automatically check PNR after issue/verify. PNR remains an explicitly selected `check` phase. The example was compiled, not executed against the supplier.

## Scope and operational limits

- The normal held-ticket issue endpoint sends no PNR request. It uses the same six supplier references, without optional fare-change/payment overrides.
- Explicit PNR reads, Cancel preflight and uncertain-ticket reconciliation remain separate operations. If NewTicket's result is lost, unavailable PNR can still prevent automatic recovery. The durable issue reservation remains blocked for investigation; this change cannot prove an unknown issue failed or authorize another issue.
- A supplier-side cancellation or deadline change may now first surface in NewTicket. A successful saved Hold is historical evidence, not a guarantee of live ticketability.
- Production ticketing restrictions and the portal Hold-only bridge remain. Removing PNR from the machine API does not add a frontend Issue action or enable real production ticketing.
- Existing unrelated workspace changes were present before this work. No real Book/Issue/Cancel, working-database migration, server restart, commit, push or deployment was performed.

## Initial verification, before the second review

- `cargo test --locked`: **71 passed** (59 library, six foundation and six production-fixture tests). Seven opt-in tests were skipped; the database test was run separately below. Live/private supplier tests were not enabled.
- Full `cargo test --locked --test database -- --ignored --nocapture`: **passed in 44.11 seconds** against a new disposable database. Its setup/migration and full suite completed successfully; the task-created database was removed afterward.
- `cargo clippy --locked --all-targets -- -D warnings`: **passed**, including the changed UAT example.
- `cargo fmt --check` and `git diff --check`: **passed**.
- Baseline before edits: 58 library tests passed and one private-evidence test was skipped.

The regressions cover exact six-field supplier payloads, zero PNR calls for issue/replay, unavailable PNR modes, failed prior lookup, omitted/unusable deadlines, explicit-offset deadlines, saved cancellation/ticket evidence, concurrent issue requests, Issue/Cancel exclusion and uncertain-outcome replay without reissue. The existing full database suite also covers ownership, permissions, accepted prices, receipt verification, report recovery, historical evidence and the portal flows.

## Second review: reproduced inconsistencies and corrections

The user requested another careful consistency review. It found:

1. **A failed PNR lookup could erase the evidence used to block issue.** Regression sequence: separately observe `Ticketed`, then receive a failed PNR response, then submit NewTicket. Before the fix the database test returned HTTP 200 instead of the required 409. Migration 0028 adds append-only PNR observations; issue selects the most recent verified response by request timestamp after locking the booking. Failed responses remain visible as failed attempts, and successful old responses finishing late cannot overwrite newer authority. Upgrade backfill and immutable-history protections are tested.
2. **Surname-only passengers could Book but not verify their issued ticket.** The focused regression failed before the fix. Identity matching now permits an exactly matching empty first name with a nonempty matching surname, preserving rejection of missing/changed surnames and ambiguous passenger sets. Integration covers Book → NewTicket → replay → ticket report with a surname-only passenger and four distinct opaque reference fields; no PNR request is sent.
3. **`Created` status had inconsistent meaning.** Ticket issue accepted it as held, while the public PNR response set `X-Manual-Resolution-Required=true`. A shared held-status predicate now gives the same interpretation for issue and status; an HTTP header regression checks the result.

Additional coverage deliberately reverses PNR completion order in both directions (older Booked/newer Cancelled and older Cancelled/newer Booked), then issues with PNR unavailable. The response with the newer request timestamp controls eligibility, independently of completion order.

Final verification of the reviewed build:

- `cargo test --locked`: **72 passed** (60 library, six foundation, six fixture tests); seven opt-in tests skipped in that command.
- Separate full PostgreSQL suite: **passed in 44.90 seconds**, including migration/backfill/immutability, opposite completion orders, single-name issue/report, distinct opaque references, zero-PNR issue/replay and existing Issue/Cancel concurrency. The disposable database was removed.
- Strict Clippy across all targets, formatting and diff checks: **passed**.
- Both reproduced defects failed before their corrections and now pass. The `Created` status/header regression also passes.

All supplier responses in these tests are simulated. No working/production migration, server restart or live issue was performed. Migration 0028 is required before running the updated build. These checks establish the covered backend behavior, not live supplier acceptance or a guarantee that every supplier itinerary is supported.

## Final flow consistency

| Step | Source/guard | Result |
| --- | --- | --- |
| Search → RePrice | Owned selected offer and its references | Live quote; no booking/issue side effect |
| Price acceptance → Book | Latest accepted owned quote; passengers; supplier gates | Saved verified Hold or unresolved outcome |
| Book → NewTicket | Saved six supplier references; local held state; latest verified optional history; explicit-offset deadline where available | One durable reservation and one NewTicket dispatch; zero PNR calls |
| NewTicket success | Matching passengers/tickets/references and accepted fare/itinerary evidence | Saved verified ticket receipt |
| NewTicket failure/timeout | Durable reservation remains | Pending/unknown; no automatic retry or second issue |
| Replay/retrieval | Ownership and exact request/key checks | Stored result without supplier issue or PNR calls |
| Optional PNR/status | Explicit request, independently recorded history | Current attempt displayed; failed/late responses cannot erase newer verified authority |
| Cancel/recovery | Existing separate capability/evidence requirements | Issue/Cancel exclusion and uncertain-result protection remain |

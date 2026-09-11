# Uncertain Issue reconciliation — 2026-09-11

Implemented the requested next reconciliation increment locally after report release.

## Result

- Client `POST /api/bookings/{id}/ticket/reconcile` requires ownership and booking/ticketing permissions. Admin unresolved queue and per-booking recheck use human administrator sessions.
- Unknown issues can be checked immediately; pending reservations wait five minutes. Verified tickets replay without supplier calls. Original supplier servicing/currency and accepted held-booking eligibility remain enforced.
- Ticketed PNR with matching references plus complete Issued report verifies passenger/ticket sets, itinerary and original fare. Existing ticket-number evidence in PNR/captured successful Issue must agree. No original Issue receipt is fabricated to bootstrap report validation.
- Original reservation/state/response remains immutable. Append-only report-backed verification provides effective issued state; row locks serialize client/Admin and saved-response verification. Late worker evidence is re-read before committing proof.
- Missing/failed/contradictory evidence stays unresolved. No reservation is released; no Book/NewTicket/Cancel is dispatched by any reconciliation path.
- Migration 0018 adds immutable attempt evidence, safe failure summary, actor and timestamps. Verification and audit commit atomically.

## Verification

- 59 ordinary tests pass (47 unit, six foundation, six fixture). One private-evidence unit test remains explicitly opt-in.
- Full disposable PostgreSQL integration suite passes in 38.91 seconds. It verifies timeout recovery without a captured Issue response, foreign ownership, machine/Admin separation, servicing gates, Search/mutation gate independence, Booked/failed/mismatched PNR, missing/wrong/duplicate passenger or ticket data, wrong fare/status/itinerary, report timeout, conflicting PNR tickets, concurrent client/Admin checks, resolved replay without reads, queue removal, immutable attempt records, fresh pending exclusion and stale pending recovery. Original issue and booking dispatch counters remain unchanged by recovery.
- Simulated late pending-worker completion preserves its original unknown outcome without erasing effective verified ticket evidence.
- Existing concurrency test fixed to inspect whichever idempotency key actually won the reservation; either racing key is valid. Stale-pending fixture setup re-enables Search only to create its independent synthetic Hold. These were test setup assumptions, not changes to public mutation authorization.
- Strict all-target Clippy, formatting and diff whitespace checks pass.

## Previously captured real UAT evidence

The opt-in offline test reads the existing private BS return report and accepted booking context from the retained local UAT database. It reconstructs the same two ticket numbers without supplying a saved Issue receipt to the recovery verifier. The result is independently compared with the prior verified ticket receipt after recovery; changed flight/RBD and changed passenger fare are rejected.

This test made zero supplier calls and zero database writes. It demonstrates compatibility with the captured real report, not a newly observed live supplier timeout incident. No intentional timeout, duplicate issue or new ticket was manufactured.

## Scope and rollout

Migration 0018 ran only in disposable test databases. Working, retained UAT and production database schemas remain unchanged by this increment. No supplier network request, real booking/issue/cancellation, Git commit/push or deployment occurred. Test databases are disposable and removed after successful verification.

Admin operations are APIs; a dedicated screen/background scheduler and manual resolution without sufficient proof are not included. Live lost-response incident acceptance, broader carrier/supplier coverage and private-evidence retention policy remain separate. Deploy through the existing backup/migration workflow when authorized.

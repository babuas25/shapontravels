# Held booking cancellation — offline implementation

Implemented and deployed on 2026-09-11. Real supplier cancellation is disabled: both cancellation and its reconciliation require application environment `test` and an offline-capable transport. Real adapters inherit unconditional denial, even with existing booking/ticketing flags enabled. No real booking was cancelled during development.

## API

- `POST /api/Cancel` uses the same six platform reference fields as NewTicket (`PNR`, `BookingRefNumber`, `BookingCodeRef`, `UniqueTransID`, `PriceCodeRef`, `ItemCodeRef`) and an ASCII `Idempotency-Key` of 1–128 characters.
- `GET /api/bookings/{id}/cancellation` retrieves the saved cancellation outcome.
- `POST /api/bookings/{id}/cancellation/reconcile` saves a read-only PNR observation. It never calls Cancel or Issue and never removes a reservation.

All routes require `booking` and `cancellation` permissions and ownership. Cancel requires a held, non-direct booking without any ticket issue reservation, original ticket evidence or ticketed PNR. A fresh, reference-verified PNR must say `Booked` with no tickets. Supplier servicing must be enabled; Search and booking activation are not required to service an existing held PNR.

## Durable outcomes

Migration `0020_held_cancellations.sql` creates immutable cancellation reservations and append-only reconciliation evidence. Reservation is committed before dispatch. One reservation per booking prevents duplicate cancellation even with a different key. Changed references or conflicting idempotency keys are rejected. Client and booking locks serialize reservations; database triggers additionally exclude an Issue reservation when Cancel exists and vice versa. An uncertain Issue also blocks Cancel, and an uncertain Cancel blocks Issue.

The Cancel transport is called once with a timeout and no retries. A detached worker persists received results if the HTTP caller disconnects. A process crash leaves the reservation pending. Replays return saved state and never send another mutation.

A verified response must have `item2.isSuccess=true`, `item1.isCancel=true`, all four echoed supplier references matching, no conflicting PNR or ticket evidence, and no nonzero or malformed payment/refund accounting. The public response contains platform references and cancellation success only. Raw supplier evidence is retained privately. This flow does not implement paid-ticket refund or void accounting.

Success returns 200. Timeout, supplier error or unverifiable response returns 202 with `state=outcome_unknown` and `requiresReconciliation=true`. Pending reservations also return 202. The original Hold receipt remains historical; Book replay and booking lookup expose `X-Cancellation-State`. Retrieve `/cancellation` for its current persisted outcome.

Reconciliation of a pending reservation is blocked for five minutes. Later read observations are saved separately, including failed reads. `verifiedCancelled=true` requires exact PNR/references, `Cancelled` status, supplier success and no tickets. This is evidence for manual review, not permission to retry: the response always has `requiresManualReview=true` and `dispatchAllowed=false`. It does not overwrite the original dispatch outcome or infer cancellation from missing PNRs.

## Validation and release status

Synthetic tests cover permissions, disabled transport, UAT/production rejection, wrong references, issued booking rejection, success, timeout, supplier failure, nonzero refund accounting, replay under the same/different key, saved retrieval, immutable evidence, read-only reconciliation and simultaneous Issue/Cancel exclusion. Real-adapter denial is tested without HTTP traffic.

Migration 0020 was subsequently applied to the backed-up local working database and production through the authorized release workflow. No live Book, Issue or Cancel test was run.

Final validation: 60 regular tests passed; the complete disposable-database suite including migration 0020 and concurrent Issue/Cancel passed in 39.09 seconds. Clippy with warnings denied, formatting and diff checks passed. All task-created test databases were removed.

## Admin review

The Admin reconciliation panel includes all cancellation records and a separate pending/unknown cancellation queue. Details show the saved cancellation outcome and up to 50 latest reconciliation evidence summaries. Raw supplier payloads, credentials and passenger documents are not exposed. Admin viewing and refreshing these records perform no supplier call. Cancellation details do not offer the unrelated manual Hold resolution form. The APIs require an Admin session.

## Verified release — 2026-09-11

Application commit `4b8bf1acae16fe51291d0643de54343920bcdf13` is deployed. [Workflow 34569830412](https://github.com/babuas25/shapontravels/actions/runs/34569830412) passed all checks, release build and production activation. Local backup archive was verified before migration 20 (successful); production completed the required backup → migration → grants → restart → health workflow. Independent production live/readiness returned 200/ok and 200/ready. OpenAPI serves Cancel and the Admin shell serves cancellation filters; unauthenticated Admin queue access returns 401.

Validation included 60 regular tests, the complete disposable-database suite (43.79 seconds), Clippy, formatting, JavaScript syntax/render checks and five deployment-script tests. The task test database was removed. Real cancellation and Direct Issue remain disabled; no real supplier mutation was tested.

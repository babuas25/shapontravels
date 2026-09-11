# Direct Issue — offline implementation

Implemented on 2026-09-11 under the user's instruction to implement Direct Issue without executing a real Issue test. This supersedes earlier statements excluding Direct Issue from implementation.

`POST /api/Book` accepts `directIssueIntent: true` only when both the saved Search offer and accepted latest RePrice explicitly have `bookable: false`. Hold requests require both values to be true. A missing, conflicting or incorrectly selected mode returns `BOOKING_MODE_MISMATCH`. Direct requests require both `booking` and `ticketing` permissions, valid owned references, accepted fresh pricing, passenger validation, matching currency, and supplier booking/ticketing/servicing gates.

Execution additionally requires the `test` application environment and a transport implementing the offline `book_direct` capability. Real supplier adapters inherit an unconditional denial, including when configured with UAT hosts and all existing enablement flags. There is no environment-variable switch to enable real Direct Issue in this release. A real integration and its authorization remain a separate release step.

A durable booking reservation is committed before dispatch. Migration `0019_direct_issue.sql` records immutable `execution_mode` and adds the `issued` booking state. The supplier Book payload contains its original references and passenger details; the platform's intent flag is not forwarded. The dedicated transport operation is invoked once, with a timeout and no retry. A process crash leaves the booking pending; replay cannot dispatch again.

A successful response must contain a nonempty PNR/booking reference, ticket reference, exact passenger matching, unique numeric ticket numbers, consistent echoed quote references, and matching fare/itinerary evidence when supplied. The existing ticket validator applies accepted selling prices and platform references. Booking outcome and immutable ticket receipt are saved in one transaction. `GET /api/bookings/{id}/ticket` retrieves the receipt; existing ticket report retrieval works with the saved servicing references. `POST /api/ticket/NewTicket` rejects a Direct Issue booking with `DIRECT_ISSUE_ALREADY_RESERVED`. The `direct:` idempotency-key prefix is reserved for internal receipt records.

Timeout, supplier failure or invalid evidence returns 202 with an unresolved booking. The original response is retained when available. Replaying the original Book key returns saved state; changing the key cannot dispatch the same offer again. A late result after a manual decision is retained separately and reopens review. Unknown Direct Issue outcomes require manual review; the existing held-ticket reconciliation flow does not authorize recovery or another Issue for them.

Validation uses synthetic in-process suppliers and disposable PostgreSQL only. No UAT or production Book/Issue calls are part of this work. The release was subsequently authorized and deployed; see the release record below.

Final validation: `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked` passed (60 tests; opt-in live/private tests skipped). The full disposable-database suite including migration 0019 and Direct Issue scenarios passed in 43.90 seconds. Temporary databases created for this task were removed.

## Release record — 2026-09-11

Application commit `0200efc127697f684f96068b1df5fffb9194f033` was pushed to main. [Workflow 34566216193](https://github.com/babuas25/shapontravels/actions/runs/34566216193) completed all checks, Ubuntu release build and production deployment successfully. The local working database was privately backed up, its archive verified, and migration 19 confirmed successful. Production activation completed its required backup, migration, grants, restart and health checks. Independent public `/health/live` and `/health/ready` checks returned 200 with `ok` and `ready`; `/openapi.json` serves the updated Book contract. Real Direct Issue remains unconditionally disabled in supplier adapters. No real Book/Issue test was performed.

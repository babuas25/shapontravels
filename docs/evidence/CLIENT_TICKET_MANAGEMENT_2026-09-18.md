# Client ticket-management endpoint verification — 18 September 2026

Implemented agency-to-Shapon-Travels request intake and customer quotation decisions on the Rust machine API. No supplier dispatch paths were added. Local release is active at rollout revision 11, migration 0052. Production was not modified.

## Behavior

- Five commercial operations: create, list, availability, detail and quotation decision.
- Explicit `ticket-management:read` and `ticket-management:write` permissions, configurable in the current dashboard. No existing client grants were modified.
- Machine credentials and permissions revalidated under the authority barrier. Exact API client ID and linked wallet owner scope apply to every booking/request lookup, including shared-wallet clients and idempotent replays.
- Reissue preferences are structured per journey, validated and saved in the immutable request snapshot. Staff see them in the existing request note; client details return the structured preferences.
- Shared admin review, quotation, customer confirmation and staff completion workflow. Intake leaves funds unchanged; accepting a debit quote reserves once. Clients cannot perform staff settlement or supplier execution.
- Existing notification outbox queues updates for the approved staff roles. Notification sending settings remain disabled and unchanged.

## Verification

Passed:

- Disposable PostgreSQL ticket-management journeys: existing financial invariants plus real-router API tests for all three intake actions, structured dates, shared-wallet cross-client denial, read/write permissions, suspended owner denial, revoked credentials, duplicate concurrent intake, changed replay payloads, active passenger conflicts, stale quote versions, staff action rejection, debit approval/replay, staff reissue completion, refund rejection and staff notification queue recipients. No supplier/provider is installed in the test router.
- Disposable PostgreSQL canonical identity/business matrix.
- Seven ticket-management rule tests covering exact money, entitlements, reissue allocations, authority and VOID cutoff.
- `cargo clippy --locked --all-targets -- -D warnings`.
- Frontend TypeScript and targeted ESLint for API-management changes.
- Diff whitespace checks in both repositories.
- Running local backend health/readiness both 200; new unauthenticated read routes 401; all five commercial operations present with machine authentication in `/openapi.json`.

## Local rollout and preservation

Paused local writers, created a private full database backup, restored it into a separate database, and verified all table/sequence fingerprints before migration. Migration 0052 changes only the allowed permission constraint; all prior business tables remained unchanged. Resumed with current binary/frontend release pins.

Post-start comparison verified 36 wallet, flight, ticket-management, notification and API-client/credential/token tables against the restored backup. No financial records or client grants changed. Email/SMS settings, sender credentials, scheduler secrets and approved deposit-alert phone configuration were preserved. No live notifications or supplier requests were sent.

Private operational evidence is in `.local/client-ticket-release/`; disposable test results are in `.local/ticket-management-tests/results.json`. These directories can contain private configuration and must not be published.

## Limits

This is local implementation and verification, not production activation. Existing booking/notification recovery backlog remains unchanged. Clients need explicit permission grants and their own eligible issued, wallet-backed booking. The recent list is bounded to 100 results; no webhook delivery or full-history export is introduced. Staff must verify supplier eligibility and perform any external airline/supplier work outside these endpoints.

Contract and examples: [Ticket-management API](../TICKET_MANAGEMENT_API.md).

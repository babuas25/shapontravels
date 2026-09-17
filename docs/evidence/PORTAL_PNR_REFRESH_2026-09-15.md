# Explicit portal PNR deadline refresh — 15 September 2026

## Result

The existing Hold receipt's **Refresh Status / Deadline** action now calls supplier PNR through the authenticated Next bridge and Rust's existing reconciliation helper. No new Rust endpoint was introduced. Normal Search → RePrice → Book → NewTicket, page load and saved receipt reload do not automatically call PNR.

The browser sends `{action:"refresh",draftId}` to `POST /api/flights/holds`. The Next handler checks same-origin POST and fresh Clerk authority, then posts the attested reader and `refresh:true` to the existing `/admin/portal-holds/receipt` route. Rust checks Super Admin/B2B ownership, active client linkage and supplier servicing before resolving stored references and performing the PNR read.

The receipt exposes only the saved verified PNR summary: status, raw valid deadline, request/check timestamp and manual-review requirement. It selects the most recent verified observation by request timestamp from migration 0028's immutable history. A later failed lookup cannot erase prior verified status or pretend it was freshly verified.

## Consistency corrections

- The frontend previously used a truthy fallback from stored deadline to original Book deadline. A verified PNR response with no deadline could therefore resurrect an obsolete Book deadline. A present details object is now authoritative, including null.
- The receipt reads latest verified history rather than the mutable latest-attempt columns. Missing/invalid latest verified deadlines clear the display; failed/mismatched responses retain the previous verified summary and its timestamp.
- PNR `MM/dd/yyyy HH:mm:ss` has no verified timezone. It is shown as supplier time; no UTC/Dhaka conversion, expiration comparison or countdown is invented. The observation timestamp is an actual UTC instant and can be displayed in Bangladesh time.
- `Booked` and `Created` share the existing held-status interpretation. Terminal/conflicting status displays manual review without silently executing a local Issue/Cancel transition. Refresh errors keep prior details visible with an explicit error.

## Verification

- `cargo test --locked`: 72 ordinary tests passed; seven opt-in/private tests skipped by that command.
- Full disposable PostgreSQL suite: passed in **44.38 seconds**. New coverage includes no PNR during Book/replay/receipt, exact saved six-reference payload, unauthenticated/machine/foreign-owner/unsubmitted/inactive-client rejection, supplier servicing disabled, successful refresh, failed and mismatched responses retaining old evidence/timestamps, missing/invalid deadline supersession, and Booked/Created/Cancelled status handling. Existing no-PNR NewTicket and concurrency checks also pass.
- `npm run verify:rust-holds`: passed, including strict refresh input/role checks, response sanitization, null deadline precedence, offset-free time handling, and Cancelled/Ticketed manual-review presentation.
- Actual Next handlers → Rust HTTP → fresh disposable PostgreSQL: passed. The fixture flow performs four Book dispatches. All ordinary actions perform zero PNR reads; two explicit refresh actions perform exactly two PNR reads and no additional Book. Owner/Super Admin reads, foreign/demoted-user denial, same-origin enforcement, safe browser projection and saved-observation reload pass.
- TypeScript, focused ESLint, Next production build, strict Rust Clippy across all targets, formatting and diff checks passed.
- Both task-created disposable databases were removed after verification.

## Run/deployment limits

All suppliers and Clerk identities in these tests were simulated. The new button was not exercised against a live supplier. No live Book/Issue/Cancel, running-server restart, working/production database migration, commit, push or deployment was performed.

Apply migration **0028** before serving the updated Rust build, then run the corresponding frontend build. This refresh addition requires no migration beyond the already pending 0028. Existing supplier credentials and servicing controls govern live operation. Frontend Issue/Cancel and merged legacy history remain separate integrations.

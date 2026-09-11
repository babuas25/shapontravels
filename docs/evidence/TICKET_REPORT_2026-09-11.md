# Ticket report implementation and UAT verification — 2026-09-11

Implemented the next requested Ticket details/report increment after the held-ticket release. This increment is local and has not been committed, pushed or deployed.

## Delivered

- Owner-scoped GET report by booking UUID, STR reference, or platform Search UUID with Confirmed filter.
- Verified ticket evidence and ticketing permission required; original supplier servicing control is independent of Search and mutation controls. Foreign/unknown records return 404; ambiguous references/transactions return 409.
- Bounded GET transport, transaction encoded as one path segment, one 401 refresh and one transient retry. No Book/NewTicket/Cancel calls.
- PNR/transaction/item reference, passenger identity, exact ticket sets, original fares, optional fare-breakdown groups and itinerary validation. Public report contains accepted selling values without another markup and excludes internal account/reference/payment/extra financial data.
- Migration 0017 stores immutable valid/rejected received report evidence and redacted audit. No issue outcome is changed or automatically resolved.

## Verification

- 58 ordinary tests pass: 46 unit, six foundation, six production-fixture tests.
- Full disposable PostgreSQL integration suite passes in 38.72 seconds. Covers ownership, permissions/servicing gates, Search/ticket-mutation independence, transaction/reference ambiguity, ticket/PNR/transaction/name/fare mismatch, duplicate passengers, lifecycle mismatch, timeout/read error, audit and immutable report evidence. Original mutation call count remains unchanged by report tests.
- GET method, URL segment escaping and bounded authentication/transient retries verified with a local mock HTTP server. Exact discount arithmetic, strict ticket-set parsing, timestamp normalization and scalar-only field projection have unit coverage.
- Strict all-target Clippy, formatting and diff whitespace checks pass.

## Actual read-only Triplover UAT

Used the previously issued BS DAC→CXB→DAC return booking, one adult and one child. The direct report and all three public route forms returned the expected confirmed report. Final public check: HTTP 200, identical payloads and matching X-Booking-Reference, two passengers, two flight segments, status Issued, accepted selling total BDT 18,382. Original supplier total is BDT 17,382. No additional markup is calculated.

The report contains CNN in passenger fare records even though NewTicket previously returned CHD in child ticket metadata. Matching uses the saved passengers and verified ticket numbers, and compares original fares before projection. Supplier referenceLog, account identity, markup/payment fields and unknown financial sections are not present in the public response.

Two earlier public harness runs reached successful HTTP responses but failed a test assertion expecting JSON decimal text `18382.00` instead of `18382`. Corrected the assertion to compare exact decimal values; the runtime price calculation was unchanged. A prior synthetic missing-reference test accidentally used an existing ambiguous reference and was corrected to a genuinely unknown reference. Final suites pass.

Private evidence:

- `.local/evidence/uat-ticket-report-20260911T042404966109000/` — direct raw report and pre-migration database backup.
- `.local/evidence/uat-public-ticket-report-20260911T042911747027000/` — final public responses for all three routes.

Raw/private artifacts use restricted permissions and stay ignored by Git. Temporary test tokens are removed and original local client permissions restored. Migration 0017 was applied only to disposable test databases and the privately backed-up retained UAT database. No new booking, Issue, Direct Issue, cancellation, production supplier request, production migration, working database migration, push or deployment occurred.

The report contract currently supports unchanged issued tickets only. Reissue/refund/cancel servicing, report evidence retention policy, missing-response Issue reconciliation and broader carrier/supplier coverage remain separate work.

## Subsequent authorized release

User requested the stated next step of migration and Git push. Commit `cd3f2b5` was pushed to main and [workflow 34562629848](https://github.com/babuas25/shapontravels/actions/runs/34562629848) succeeded through checks, build and production deployment. Migration 0017 ran after backup through the existing workflow. Independent public HTTPS checks returned liveness/readiness 200, confirmed all three report routes in OpenAPI, and verified unauthenticated report access returns 401. Local working database backup/migration also succeeded. No supplier calls, ticket mutations or UAT data transfer were part of release verification. Earlier local-only statements above describe the implementation stage.

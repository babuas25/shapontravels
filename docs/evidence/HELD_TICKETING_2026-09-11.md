# Held booking ticket issue — 2026-09-11 (Asia/Dhaka)

User explicitly authorized confirming holdable bookings through Triplover UAT and excluded Direct Issue. This work implements that scope locally; production execution remains blocked.

## Implementation and verification

- NewTicket endpoint with platform PNR references, owner and booking/ticketing permissions, accepted holdable fare, fresh PNR and conservative deadline checks. Database/transport gates and exact UAT hosts are enforced.
- One durable issue reservation per booking, including concurrent requests with different keys. No retries on supplier 401/5xx/timeout, retained unknown outcomes, background completion and redacted audit.
- Stored ticket retrieval, original Hold response retention and `X-Ticket-State`. Append-only captured-success verification after compatibility corrections; no supplier call from that endpoint.
- 55 ordinary tests pass (43 unit, six foundation, six fixture). Full disposable PostgreSQL integration suite passes in 39.60 seconds. Strict all-target Clippy, formatting and diff whitespace checks pass.
- Integration covers permission/transport/database/production gates, foreign ownership, reference mismatch, nonholdable fare, invalid PNR/status/deadline/ticket evidence, expired Search quote on a valid Hold, concurrency, key conflicts, immutable evidence, passenger/PNR/fare mismatch, missing tickets, supplier failure/timeout and saved-response revalidation without another issue.

## Actual UAT result

Used the existing isolated BS return Hold for DAC→CXB→DAC, 10 / 13 November 2026, one adult and one child. No new Search/RePrice/Book was required or sent.

1. Backed up retained `shapon_bs_return_uat_booking_test` privately before migration 0015. An initial probe token-format error returned local 401; no supplier request was dispatched by that failed probe. Fixed the harness token construction.
2. Fresh public PNR lookup returned HTTP 200, Booked, deadline `09/12/2026 22:45:42`. The deadline passes the conservative earliest-UTC bound without assuming the supplier timezone.
3. Exactly one `NewTicket` dispatch. Supplier `item2.isSuccess=true` with two distinct 13-digit ticket numbers. A subsequent PNR read returned HTTP 200 / Ticketed.
4. Initial public result was 202/outcome_unknown because booked CNN was labelled CHD in ticket passenger metadata. Exact passenger names and references matched, as did flight passenger counts, itinerary, RBD, and passenger/component original money. No price rewrite or second Issue was used.
5. Added directed Triplover CNN→CHD compatibility only with matching names and unchanged flight counts/fares; other supplier/type mismatches remain rejected. Public ticket metadata preserves the returned CHD label.
6. Backed up again before migration 0016. Public saved-evidence verification returned HTTP 200 with two ticket passengers; saved ticket retrieval is identical. A further PNR read remains Ticketed.

Independent Decimal check: original supplier total BDT 17,382; retained accepted selling total BDT 18,382. Markup was not added again.

Database audit confirms exactly one issue reservation, one dispatch audit and one append-only verification. Original outcome_unknown response evidence remains intact; effective ticket state is issued. Local permissions restored to search:read/booking, ticketing control restored disabled, servicing retained enabled, and zero active probe tokens remain.

## Private evidence and boundaries

Private capture paths use UTC timestamps on 2026-09-10; the session date in Asia/Dhaka is 2026-09-11:

- `.local/evidence/uat-held-ticket-check-20260910T205041150268000/`
- `.local/evidence/uat-held-ticket-issue-20260910T205109359387000/`
- `.local/evidence/uat-held-ticket-verify-20260910T205321569468000/`

Captures and backups are restricted local files; passenger information, opaque references and ticket numbers are not copied into this report or source control. The retained UAT database remains available for continuation.

No Direct Issue, cancellation, new Hold, production supplier mutation, working/production database migration, commit, push or deployment. Migrations 0015–0016 were applied only to isolated test/UAT databases. Production commercial authorization, ticket reports and resolution without captured successful ticket evidence remain future work. This verifies a representative BS return flow, not every carrier/fare/supplier.

## Subsequent authorized release

User subsequently authorized database migrations and Git push. Application commit `08a569e` was pushed to main and [workflow 34561334396](https://github.com/babuas25/shapontravels/actions/runs/34561334396) completed successfully, including checks, build and production backup/migration/deployment. Public HTTPS liveness/readiness returned 200 and all three ticket endpoints appeared in served OpenAPI. Migrations 0015–0016 were also applied to the backed-up local working database. Earlier local-only statements above are historical. No supplier mutation or private UAT data transfer occurred during this release.

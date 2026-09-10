# Admin browser QA and public Triplover UAT booking flow — 2026-09-10

## Scope

User authorized continuing browser validation and Triplover UAT booking/PNR/reconciliation verification. Production Hold/Issue remains prohibited. The flow dispatched exactly one UAT Book; no ticketing or Cancel. A later user interruption did not authorize a new booking: continuation resumed reads/browser checks only.

## Browser result

Passed on the actual Rust-served page in Codex's browser, using only synthetic database records and a PNR mock:

- Incorrect credentials produce visible Bengali feedback; valid existing Admin credentials open the queue.
- Booking selection shows details; status recheck refreshes supplier status/deadline without booking.
- Submitting an empty outcome shows native required-field validation.
- Evidence, reason, support reference and confirmation checkbox submit successfully. A disposable synthetic booking leaves the unresolved queue and shows the saved Admin/time/outcome/evidence.
- Resolved filter works; two-entry history preserves the earlier decision after the late-response test.
- Missing supplier references show the expected supplier portal/support guidance.
- Logout clears booking content; page reload requires login again.
- Default-viewport screenshots were inspected for readable Bengali layout. No mobile breakpoint or exhaustive browser compatibility claim.

A small issue found during QA was fixed: the heading and explanatory text now follow the unresolved/resolved filter. This final change was rechecked in-browser after restarting the synthetic server. The user interruption stopped the earlier server; the isolated server was restarted without replaying any supplier booking. Final synthetic server stopped cleanly and its database was removed.

Harness: `tests/admin_browser.rs`, ignored unless explicitly opted into a local database whose name ends in `_admin_browser_test`. It never loads supplier credentials and implements only a PNR mock. Reuse of existing synthetic data requires an additional explicit test flag.

## Configuration correction

The supplied `.env` had a non-assignment heading on line 19 that was not a comment. The initial UAT runner stopped before any network use because no UAT host was loaded. Converted only that heading to a comment; host/credential assignments were preserved. The runner now requires successful dotenv parsing rather than silently ignoring errors.

Before any supplier traffic, assertions verify HTTPS, the exact Triplover UAT Search/API hosts, standard HTTPS port and root path. No production supplier is instantiated by the UAT runner.

## Public UAT flow

Used the existing authorized private UAT passenger input, DAC–SIN on 1 November 2026, one adult and one three-year-old child. Local isolated B2B pricing used fixed 500 per passenger; no working/production pricing configuration was changed.

| Stage | Result |
|---|---|
| Public Search | HTTP 200, 36 offers |
| Public RePrice | First selected refundable/bookable candidate passed HTTP 200; bookable=true and no selling-price change |
| Local price acceptance | HTTP 200 |
| Supplier Book | Exactly one dispatch; `isSuccess=false`, `item1=null`; business message reports duplicate booking for the passengers |
| Public Book | HTTP 202, `outcome_unknown` durably recorded |
| Same-key replay | Identical stored HTTP/body; supplier Book call count remains one |
| Client reconciliation | HTTP 409 `MANUAL_RECONCILIATION_REQUIRED`; no PNR/reference returned by Book |
| Admin recheck | HTTP 409 `MANUAL_RECONCILIATION_REQUIRED`; no guessed refs or new Book |
| Manual resolution | Not performed on the real UAT attempt; duplicate message is not sufficient proof to assert created/not-created |

The public `/api/pnr` six-field success branch could not be exercised for this attempt because no PNR was returned. The existing client reconciliation and Admin recheck correctly stopped before an upstream PNR call.

As a separate **read-only diagnostic**, used the original six references from the previously captured 2026-09-08 successful UAT hold. Supplier again returned `isSuccess=false` with `Record locator not found`, no current status or deadline. This does not establish that the old reservation is absent/cancelled, that it is the duplicate's exact source, or that the supplier has a software bug. Supplier account/session/reference lifecycle and lookup expectations need clarification.

No duplicate message containing passenger names was included in public API output or this report. Raw requests/responses and the old-PNR diagnostic remain private.

## Persistence and checks

Private evidence: `.local/evidence/uat-public-booking-20260910`. Private database archive `isolated-database.dump` was saved with mode 0600 under the private evidence directory; `pg_restore --list` confirmed booking/audit entries before the disposable UAT database was removed. Archive SHA-256: `4b207b7a3a9d84ce0a96e2269b1df33657c92363ae59cf4258efb4a06b4940e8`. No restore validation is claimed. Restore this isolated archive if further local investigation of its booking record is required.

All 46 ordinary tests, strict all-target Clippy, Rust formatting, JavaScript syntax and diff whitespace checks pass. The synthetic browser fixture setup also completed the authentication/booking integration assertions, and the browser server exited cleanly. UAT examples remain explicit opt-in tools with durable create-new evidence guards; they are not ordinary tests and must not be rerun as booking retries.

No production database changes, production Hold/Issue, Cancel, NewTicket, commit/push or deployment occurred.

## Outstanding supplier verification

Ask Triplover UAT support to identify the duplicate reservation for the submitted test passenger/route/date, confirm its current status and explain how to retrieve it through PNR using the documented six fields. Reconcile the current unknown intent from that evidence before treating it as failed or complete. Any new test booking must be a separately intentional request, not an automatic new-key retry of this unknown intent.

Browser workflow and the UAT failure/idempotency path are verified. A successful public UAT Hold → live PNR → reconciliation flow remains unverified, so Step 6 is not complete. Cancel/ticketing remain separate pending implementation/acceptance work.

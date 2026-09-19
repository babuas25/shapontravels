# Production client API Hold audit — 18 September 2026

Follow-up: the operator confirmed PNR `19 Sep 2026, 12:44 PM` as Bangladesh local time and the preferred deadline. A live PNR refresh now preserves it as `2026-09-19T12:44:00+06:00`. The user also corrected `.env`; subsequent parsing passed. See [three-supplier and deadline verification](CLIENT_API_THREE_SUPPLIERS_2026-09-18.md). The findings below describe the earlier snapshot.

**Result: production Search → FareRules → Reprice → acceptance → Book → saved status → PNR succeeded. Exactly one Hold was dispatched; Issue and Cancel were not dispatched.**

The user authorized a real production Hold for a flight one month later and explicitly prohibited Issue. The updated Triplover credentials were used against `https://apiv2.triplover.com/` and `https://api.triplover.com/`. Requests passed through the current local client API router with `environment=production` and an isolated local database. This is live supplier acceptance evidence, not a deployed HTTP/proxy/load test. The running application and its database were not changed or restarted.

## Reservation and request/response results

- Flight: **US-Bangla BS 103, DAC → CGP, 18 October 2026, 12:00–12:55**, as returned by the supplier. Departure is one calendar month after the test date.
- Local booking ID: `f67ebb85-4ae9-4804-8348-ab3914056e54`.
- Public booking reference: `STR0AD74R0AD74R`.
- Client payable quote: **BDT 4,144.61**, under isolated audit pricing rules. This is a quote, not a wallet debit.
- Passenger name was supplied by the user; the date of birth was test data. Passport number/expiry were left empty for this domestic test. Existing project booking contact/account email were used. Raw passenger/contact data and credentials remain private.

| Operation | Result |
| --- | --- |
| Search | 200; 7 BS offers |
| FareRules | 200 |
| Reprice and acceptance | 200, including a fresh pair immediately before Book |
| Book | 200; supplier `isSuccess=true`, `bookingStatus=Created`, PNR present |
| Same-key Book replay | 200; identical status/body; no additional supplier dispatch |
| Saved booking | 200; identical receipt |
| PNR | 200; `status=Booked` |
| Supplier GlobalSearchB2B | 200; one matching transaction, `Booked` |
| Supplier AirTicketingDetails | 200; `Booked`, `statusFor=Booking`, `isCompleted=true`, PNR present, no ticket number |

Counts: **1 supplier Book, 0 Issue, 0 Cancel, 0 local wallet ledger entries**. No ticket reservation or cancellation row exists in the isolated database. The real production Hold is retained; this audit did not ticket or cancel it.

## Findings

### Deadline values disagree and omit timezone

The Book response returns `ticketingTimeLimit="19/09/2026 14:44:35"`. The verified PNR response returns `lastTicketTime="19 Sep 2026, 12:44 PM"`. These are different displayed deadlines on 19 September, approximately two hours apart. Neither includes a timezone. The report's `lastTicketingTime` is null.

No timezone conversion, effective deadline, or root cause is inferred. This may reflect distinct upstream deadline rules; supplier clarification is required before clients treat either value as authoritative. The API preserves the original values. Successful Hold/PNR does not establish Issue readiness, and Issue was outside this run's authorization.

### Current `.env` fails normal application parsing

The updated `.env` has parser errors at lines **8 and 14**. A startup-path check using an intentionally unsupported CLI argument exited with **`could not parse .env`**, before serving or connecting to a database. This confirms a configuration defect independent of the successful supplier calls.

For the audit only, the launcher loaded the required parsed Triplover values into the child process, asserted exact production hosts, selected the isolated database and explicitly set production mode. The `.env` file was not modified. Its syntax must be corrected before a normal application startup using this file will work. No credentials or invalid-line contents are included in this report.

## Issue prohibition and validation

Production has a separate audit transport exposing only read operations and Hold. It inherits disabled Issue, Direct Issue and Cancel methods; it cannot call the underlying ticket methods. The adapter's ticket flag is forced off in the audit process, the isolated supplier database gate is off, and the audit client lacks ticketing permission. There is no wallet funding step. A new private request capture and existing-booking check prevent a restarted probe from silently making another reservation.

An offline regression test enabled the underlying adapter's ticket flag deliberately and verified that the production wrapper still denies Issue/Direct Issue/Cancel without dispatch. The user-owned `.env` ticket flag remains unchanged.

- Read-only prebooking probe: passed; **0 Book** dispatched.
- Authorized Hold probe: passed; **1 Book**, exact replay and saved receipt, successful PNR, no ticket/cancel/wallet records.
- Offline prohibition regression: passed.
- **11 public response captures / 15 fare-breakdown snapshots** validated against the served client contract and exact decimal fare equations; zero failures.
- Targeted audit Clippy with warnings denied, formatting and diff checks passed. Application source and migrations were unchanged in this production-testing step. Earlier full application regression evidence remains in the [Issue follow-up](CLIENT_API_ISSUE_FOLLOWUP_2026-09-18.md).

Private evidence: `.local/client-api-audit-20260918/production-hold-evidence/`, including a receipt, raw captures and isolated database archive. Archive listing was checked; restore rehearsal is not claimed. The audit PostgreSQL cluster was stopped after archiving. The supplier's real Hold remains unaffected by stopping this local cluster.

Machine-readable results: [verification manifest](CLIENT_API_PRODUCTION_HOLD_VERIFICATION_2026-09-18.json).

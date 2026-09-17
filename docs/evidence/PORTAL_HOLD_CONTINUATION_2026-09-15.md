# Book & Hold continuation — 15 September 2026

**Later route checks:** Chattogram, Bangkok and Kuala Lumpur searches succeeded, as did Bangkok return and domestic multicity searches. A fresh Bangkok return completed as a real UAT Hold for Nadia Rahman, PNR `0A4OYD`, payable BDT 41,932, with supplier PNR status `Booked` and no ticket issue. The Bangkok adult/child/infant search succeeded, but both selected family fares timed out during Hold preparation. See [the alternate-route UAT record](PORTAL_ALTERNATE_ROUTES_UAT_2026-09-15.md) for cabin/route-display findings and current coverage. The earlier failures and database counts below describe this earlier continuation only.

## Scope

Frontend: `/Users/ashifbabu/Projects/shopontravels`. Rust backend: `/Users/ashifbabu/Projects/shapontravels`. KaligonjTours was a read-only passenger-validation reference. The production Shapon site was not used for booking. No ticket issue, commit, push or deployment was performed.

## International UAT blocker

Both searches used the actual local signed-in frontend at localhost:3000 and Rust UAT bridge on 18081, with the existing updated Triplover UAT configuration.

| Search | Date / passengers | Result |
| --- | --- | --- |
| DAC → SIN | 30 September 2026 / 1 adult | Frontend: Flight suppliers are temporarily unavailable. Direct authenticated Rust `/api/Search`: HTTP 503 `ALL_SUPPLIERS_FAILED`, 120.3 seconds, no offers. |
| DAC → CCU | 30 September 2026 / 1 adult | Frontend: Flight suppliers are temporarily unavailable. |

Private raw SIN error evidence is retained in ignored `.local/evidence/portal-uat-extended-20260915/dac-sin-search.json`. No Hold could be submitted from these searches. International, return/multicity and child/infant supplier coverage is not claimed.

The browser prebooking client now bounds Search at 240 seconds and other prebooking reads at 180 seconds, including stalled response bodies. Timeout produces an actionable error without automatic retry. A regression simulates both stalled fetch and stalled response-body cases.

## B2B self-booking connected

- Fresh Clerk role and active account checks at Next; owner equality enforced again in Rust.
- B2B can prepare only its own search offer. Staff and foreign B2B offers are rejected.
- The same explicit owner-tier price acceptance and passenger form lead to native Hold.
- B2B saved passengers stay in the own-profile scope. It cannot list assignees or provision clients.
- Creator equals owner for self-bookings. Ticket table labels show the agency instead of Admin.
- Existing draft idempotency handles concurrent/repeated submit once and recovers unknown outcomes without redispatch.

The end-to-end regression uses actual Next handlers, Rust HTTP and PostgreSQL. Clerk identities and supplier responses are simulated. Four simulated Book dispatches cover three held records and one uncertain response; they are separate from the UAT database. This does not prove a real signed-in B2B supplier Hold.

## Remaining items

1. Supplier UAT: retry extended route/passenger coverage when the supplier search succeeds; current Rust search does not offer INS.
2. Receipt Refresh still reads stored state. The existing native Admin PNR recheck is read-only but has not been connected to an owner-authorized portal refresh action.
3. Legacy history is not merged into the Rust Ticket list. A scoped migration/read plan is needed before historical records can share its pagination and filtering.
4. Offers with more than 12 route combinations are still omitted by the existing mapper. This is a code-confirmed gap, not an explanation established for the supplier's 503.
5. Creator sorting uses stored IDs; local agency metadata may be missing.

## Completed checks

- Next production build and TypeScript: passed; focused ESLint: passed.
- Frontend Hold, prebooking timeout and checkout-validation regressions: passed.
- Next → Rust HTTP → fresh isolated PostgreSQL Hold suite: passed, including self-booking creator labels and supplier-price privacy. The concurrency test accepts the legitimate pending response from a retry while the winning request completes, then verifies the final held receipt and single dispatch.
- Rust library: 58 passed, one pre-existing private-evidence test ignored. Full PostgreSQL suite: passed. Strict Clippy: passed.
- Both repositories pass `git diff --check`.
- Local Rust UAT server rebuilt and restarted on 18081 with the existing database/configuration and ticket issue disabled; readiness reports `environment=test`. The original Ticket table still displays the existing UAT Hold after the restart.

The UAT database was rechecked after this continuation: one existing Hold, zero ticket issues and zero ticket verifications. No additional actual supplier Book was sent.

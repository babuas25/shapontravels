# Search → FareRules → RePrice review — 2026-09-14

Scope: correct the Rust backend and its existing Next frontend integration locally, preserve the current design, and stop before booking/wallet. No commit, push, deployment or remote database write was performed. The running application is localhost:3000 → Rust 127.0.0.1:18081, using the isolated `api_portal_local_clerk_20260914` database. Saved passenger migration is independent of this flow.

## Guideline sources

- [Prebooking flow](../PREBOOKING_FLOW.md), [Search/FareRules](../SEARCH_API.md), [RePrice](../REPRICE_API.md), [portal boundaries](../PORTAL_PREBOOKING.md).
- [FareRules contract investigation](FARE_RULES_CONTRACT_INVESTIGATION_2026-09-10.md) and [RePrice contract recheck](REPRICE_CONTRACT_RECHECK_2026-09-10.md).
- Sibling frontend `Triploaver_API_Documentation.md`, sections 4.2–4.4 and reference table; `docs/RUST_PREBOOKING.md`.

## Findings corrected

| Area | Problem | Correction |
|---|---|---|
| Rust FareRules lifetime | Only the offer expiry was checked | Check both offer and parent Search before and after the supplier read |
| Rust FareRules supplier configuration | Disabled/reconfigured suppliers could receive requests using older offers; hardcoded timeout | Enforce saved supplier, availability epoch and currency; use its configured timeout; recheck availability after the read |
| Rust FareRules response | Malformed successful rule envelopes could be returned; adapter timeout was treated as generic failure | Validate rule-array shape; report malformed rules as 502 and all supplier timeout paths as 504 |
| Rust RePrice conditions | Missing `bookable`/`refundable` flags could become a usable revision | Require all three boolean flags, including `isPriceChanged`; preserve valid false values |
| Rust price lifetime | Parent Search could expire during RePrice; revision/acceptance only tracked a narrower set of expirations | Recheck Search after the call, cap revision lifetime at the earlier Search/offer expiry, and check all three lifetimes on acceptance |
| Frontend error recovery | Throwing plain messages lost actionable Rust error codes | Preserve status/code; offer fresh Search, same-airline alternatives, or RePrice as appropriate |
| Frontend quote validation | RePrice selection identity/currency and updated refundability were incompletely checked/displayed | Check Search + offer IDs and currency; show refreshed refundability and hold conditions |
| Frontend empty state | Offers excluded by presentation could be reported as no flights | Say options could not be displayed and suggest a narrower search |

These corrections require no additional schema migration. The PostgreSQL contract test is `tests/support/prebooking_contract.rs` and runs inside the complete database suite.

## Verified flow

1. Search retains all returned platform offers in the backend and Next bridge, with owner-scoped exact pricing snapshots. The frontend groups schedules and retains each displayed fare's own references under Select.
2. FareRules is optional and uses one complete selected direction per route, in route/segment order. RePrice uses the same original Search selection and saved supplier. No mixing, automatic supplier switch or invented rule text.
3. FareRules business/transport/malformed-response failures do not invalidate a quote or block user-initiated RePrice. Expired references and changed supplier availability require new Search.
4. Successful RePrice retains null/missing refreshed segment refs, returns a new price revision, and displays latest passenger totals, refundability and `bookable`. Original Search refs remain the input to later FareRules/RePrice calls.
5. `bookable=false` remains reviewable; it may require immediate issue at a later booking step. This flow performs no booking or ticketing.
6. Every B2B revision requires explicit local acceptance, even with unchanged totals. Stale, foreign, tampered, expired or superseded revisions are rejected. Admin/Super Admin see supplier/gross and cannot accept a B2B price.

## Verification

- `cargo test --locked`: ordinary tests passed; opt-in suites retain their gates.
- Complete PostgreSQL suite passed in fresh `api_prebooking_contract_test_20260914` (42 seconds). Includes supplier changes/expiry during calls, malformed conditions/rules, valid null segment refs, price lifetime and acceptance checks, with no booking rows created by the prebooking contract test.
- `cargo clippy --locked --all-targets -- -D warnings`, formatting, and example builds passed.
- `node scripts/verify-rust-prebooking.mjs`: original selection before/after RePrice, optional FareRules failure, null refreshed refs, explicit unchanged-price acceptance, identity/currency validation, exact money, 101-offer pricing batches, grouping, and rejected-offer removal passed.
- Real Next handlers → Rust HTTP → PostgreSQL passed in fresh `api_prebooking_local_contract_20260914`, using `--http-only` on port 18084. Clerk and supplier responses were simulated; HTTP handlers, Rust and database were real. All three tiers and both staff roles passed. No external requests or booking rows.
- Frontend TypeScript, ESLint and optimized build passed.
- Real signed-in Chrome page on localhost:3000: DAC–SIN, September 30, one adult. Rust saved 314 offers from FirstTrip/TakeOff; the existing UI grouped them into 131 schedules. None exceeded the presentation combination limit for this Search.
- Live TakeOff FareRules was unavailable for the selected IndiGo fare. RePrice still completed: supplier BDT 27,531.00 → 27,570.00; gross BDT 28,031.00 → 28,070.00. The original card showed changed-price review, non-refundable and hold-supported conditions, and staff review-only text.
- A second live FareRules read for US-Bangla BS-309 succeeded, displaying exchange penalties, refund penalties and general conditions as original multiline supplier text in the existing policies panel. The desktop layout was visually checked.
- Expired only that newly generated local Search. FareRules returned the expired-reference message; the UI cleared the quote and offered Search again. Clicking it produced a new Search ID and new results. The local `flight_bookings` count remained zero.

## Remaining review finding

The existing frontend mapper omits an offer when its route alternatives exceed **12 complete combinations**. Rust and the Next Search response retain it, but it cannot be selected in the current UI. This intentionally preserves the earlier production mapper's bounded expansion and is not full compliance with displaying every returned option. Removing the limit safely needs lazy/paged complete selections while preserving grouping and exact references; simply removing the bound risks a large Cartesian expansion. The limitation is now explicit here and the all-omitted state no longer claims no flights exist.

Live supplier availability and rule support can change. Mock contract coverage is not evidence that every supplier/airline supplies FareRules. Booking, ticketing, wallet settlement and legacy booking-resume migration remain outside this increment.

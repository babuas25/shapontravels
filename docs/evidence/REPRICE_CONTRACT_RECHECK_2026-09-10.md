# Supplier-confirmed reference contract and live recheck — 2026-09-10

## Outcome

After removing our incorrect requirement for RePrice response segment references, all sampled roundtrip and multicity RePrices passed. Triplover 6E passed with six null segment references; Triplover TK passed both samples without the previous duplicate-key error. Across the main matrix and focused Takeoff BS/MH run, 35/38 RePrices passed. Three one-way supplier fare errors remain. No booking or ticketing calls occurred.

## Contract correction

The user supplied the supplier's clarification: Search returns item code and segment refs; RePrice requests use those Search refs; RePrice returns item code and price code; Book requests use the RePrice item/price codes. Response segment refs are not required.

Removed only the mandatory RePrice response segment-reference check. Selected Search refs still determine the requested directions and are forwarded unchanged to the same supplier. Returned flights, passenger mix, currency, required transaction/item/price references, prices, ownership, expiry and enablement are still validated. Null segment fields remain null; no Search-reference fallback is injected into the response. Existing Book payload already uses refreshed transaction/item/price references and has no segmentCodeRefs field; it was inspected but not executed against suppliers.

Earlier reports treating null RePrice segment refs as a supplier defect or blocker requiring a supplier change are superseded by this contract. The public 422 was due to our implementation assumption.

## Production read-only scope

Current local public router against production Firsttrip, Takeoff and Triplover, with new isolated PostgreSQL databases. Same dates/routes as prior verification: DAC–SIN 15 October 2026; return SIN–DAC 20 October; multicity DAC–SIN 15 October / KUL–DAC 20 October / DAC–MLE 25 October. Two adults, children aged 3 and 11, one infant. Trusted BDT configuration and fixed BDT 500 All/All markup per passenger in the isolated test databases.

Each itinerary ran with all three suppliers, then each separately. Two offers per retained supplier were repriced after each successful Search. A focused Takeoff multicity run added BS/MH coverage. Fresh Search references were used, not the user's expired Postman references. No production server, main database, environment or supplier activation settings were changed. The local PostgreSQL service was started for the test databases.

| Itinerary | Scope | Search offers (HTTP 200) | RePrice passed / samples |
|---|---|---:|---:|
| Oneway | All | 562 | 4/6 |
| Oneway | Firsttrip | 170 | 1/2 |
| Oneway | Takeoff | 136 | 2/2 |
| Oneway | Triplover | 262 | 2/2 |
| Roundtrip | All | 2,223 | 6/6 |
| Roundtrip | Firsttrip | 456 | 2/2 |
| Roundtrip | Takeoff | 658 | 2/2 |
| Roundtrip | Triplover | 1,109 | 2/2 |
| Multicity | All | 767 | 6/6 |
| Multicity | Firsttrip | 65 | 2/2 |
| Multicity | Takeoff | 174 | 2/2 |
| Multicity | Triplover | 510 | 2/2 |
| Multicity | Takeoff BS/MH focus | 177 | 2/2 |

All 13 Search calls returned 200 with X-Search-Partial=false. Main matrix: 33/36 RePrices passed in 354.04 seconds. Focus: 2/2 passed in 28.45 seconds. In total, 12 successful RePrices contained null segment references and were accepted under the confirmed contract. No SUPPLIER_REFERENCE_MISSING errors occurred.

## Remaining errors

All three failures returned supplier isSuccess=false and public 502 SUPPLIER_REPRICE_FAILED:

- Takeoff BS oneway, all-supplier run: `000747 NO VALID FARE FOR INPUT CRITERIA`.
- Triplover BG oneway, all-supplier run: `NO COMBINABLE FARES FOR CLASS USED`.
- Firsttrip BG oneway, Firsttrip-only run: `FLIGHT SEGMENTS UNAVAILABLE IN THE REQUESTED CLASS`.

Other samples for those supplier/carrier pairs passed. These are sample-specific upstream fare responses; the underlying cause is not established. Triplover TK's previous duplicate-key error was not reproduced in either fresh sample, which does not prove all TK inventory is fixed.

## Verification and evidence

- Local ordinary suite: 21 unit tests, 3 foundation tests, 6 production-fixture tests passed. Disposable PostgreSQL integration suite passed, including null-response-reference success and persistence, unchanged selected refs, and rejection when item/price codes are missing.
- Formatting, all-target Clippy with warnings denied and diff whitespace checks passed.
- Independent Decimal audit: 7,281 raw Search offers with zero pricing discrepancies; all 7,269 retained selling offers and all 35 successful RePrices passed per-passenger HALF_UP fixed-500 pricing and aggregate/component checks. Inventory is repeated across calls; these are not unique-offer counts.
- Main/focus databases persisted 33/2 pricing revisions, all with selected directions, and zero bookings.
- Private raw requests/responses, public responses, snapshots and independent audit files: `.local/evidence/reprice-contract-20260910-5pax/` and `.local/evidence/reprice-contract-mh-20260910-5pax/`.

Only supplier Search/RePrice operations were allowed by the live harness; booking/ticketing flags were false. No FareRules, acceptance, Book, Cancel, PNR or ticketing execution was included in this live run. The implementation is local and has not been deployed. Broader workflow acceptance remains open.

## Follow-up: alternative offer selection

The user requested selecting other offers after fare rejection. A fresh isolated read-only run used the same DAC–SIN date and five-passenger mix, each supplier separately. The diagnostic skipped the cheapest returned offer, tried another offer with its own references and stopped at the first successful RePrice (bounded to five candidates). All three suppliers passed on the first alternative attempt: Firsttrip BG, Takeoff BG and Triplover BG, each HTTP 200. Searches returned 169, 137 and 267 offers respectively. This is a fresh inventory check, not a claim that a previously expired offer recovered.

The independent Decimal audit passed for all 573 saved selling offers and three successful RePrices. Capture completed in 28.96 seconds; evidence is under `.local/evidence/alternatives-20260910-5pax/`. No booking, local price acceptance or deployment occurred. Alternative iteration is an opt-in diagnostic harness mode, not a new automatic production supplier-switching policy. A client choosing another offer must RePrice its own references and accept that price before any future booking.

# Aggregate Search summary verification — 2026-09-10

## Implementation

Metadata now describes the retained, marked-up offer set rather than the first supplier's inventory. Net fields use payable group totals; gross fields independently summarize unchanged passenger Base+Tax+AIT. AIT belongs to the net-extremum offer and must not be added twice. Counts mean fare offers, airline groups use plating carrier, supplier count means represented sources, and stops are the union of reported direction stop counts. Existing null/missing fields and unknown metadata remain preserved. Unsupported populated summary data rolls back search persistence.

See [Search contract](../SEARCH_API.md) for field definitions, deterministic row/tie handling and the single-response pagination boundary. No database migration is added.

## Local verification

Formatting, strict all-target Clippy, 26 unit tests, 3 foundation/HTTP tests and 6 production-fixture tests passed. Four new summary tests cover cross-source airline templates, exact decimals beyond binary floating-point precision, passenger-group gross/AIT, return/multicity, empty/null/missing fields and malformed metadata. Disposable PostgreSQL integration passed, including all seven supplier subsets, post-selection counts, partial failure and rollback on summary errors.

## Production read-only validation

Current local public router against all three production Search accounts, using an isolated database and fixed BDT 500 per passenger. One-way DAC–SIN 15 October 2026; return SIN–DAC 20 October; multicity DAC–SIN / KUL–DAC / DAC–MLE on 15/20/25 October. Two adults, children aged 3 and 11, one infant.

All 12 Search scenarios returned 200, `X-Search-Partial: false` and `X-Search-Summary-Scope: retained-selling-offers`. Completed in 221.39 seconds. An independent Python Decimal audit verified all 7,207 returned offers' summary extrema, AIT, airline counts/minima, stop union, source counts, fixed-500 passenger markup, envelope keys and unchanged unrelated metadata. Counts include repeated inventory across scenarios, not unique flights.

| Scenario | Returned offers | Airlines | Retained sources |
| --- | ---: | ---: | ---: |
| multicity-all | 776 | 5 | 3 |
| multicity-firsttrip | 65 | 5 | 1 |
| multicity-takeoff | 183 | 4 | 1 |
| multicity-triplover | 528 | 4 | 1 |
| oneway-all | 585 | 16 | 3 |
| oneway-firsttrip | 170 | 15 | 1 |
| oneway-takeoff | 170 | 12 | 1 |
| oneway-triplover | 267 | 16 | 1 |
| roundtrip-all | 2177 | 14 | 3 |
| roundtrip-firsttrip | 459 | 12 | 1 |
| roundtrip-takeoff | 740 | 11 | 1 |
| roundtrip-triplover | 1087 | 13 | 1 |

Private responses, saved snapshots and reproducible audit: `.local/evidence/search-summary-20260910/`. The harness's new `SEARCH_MATRIX_SEARCH_ONLY=yes` mode skips RePrice/FareRules/acceptance; no supplier Book/Cancel/Issue/PNR call occurred. Main database and supplier controls were not changed by this test. This local-router validation is distinct from post-deployment HTTPS validation, which is recorded after release.

Broader canonical equivalence, complex scoped markup, branded/multiple-component fares and remaining booking/ticketing requirements remain open.

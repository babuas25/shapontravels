# Requested production Search / RePrice audit — 2026-09-09

Local project execution against production Firsttrip, Takeoff and Triplover APIs. No Book, Cancel, Issue, payment or ticketing calls were made. Runtime code and deployment were not changed.

## Scope and configuration

- Oneway: DAC → SIN, 2026-10-15.
- Roundtrip: DAC → SIN, 2026-10-15; SIN → DAC, 2026-10-20.
- Multicity: DAC → SIN, 2026-10-15; KUL → DAC, 2026-10-20; DAC → MLE, 2026-10-25.
- User-confirmed passengers: 2 adults, 2 children aged 3 and 11, 1 infant. Supplier counts: ADT 2, CHD 1, CNN 1, INF 1.
- Isolated local database and temporary client/admin; BDT; All/All fixed markup BDT 500 per passenger (BDT 2,500 for the group); 120-second supplier timeouts. These are audit settings, not a verification of the deployed database's markup configuration.
- Base revision: `4be3d9f8101310dbcc784781bc704f8a9982ebdf`, with diagnostic harness changes. Active production environment settings were retained; mutation flags were disabled in the test process.
- Twelve public Search scenarios: each itinerary with all suppliers, then each supplier separately. Immediately following each successful Search, two offers were repriced: lowest total and a different-carrier sample where available.

## Public Search results

Cells show HTTP status and returned offer count. Every 422 below was `SUPPLIER_PRICING_COVERAGE_UNSUPPORTED`.

| Itinerary | All three | Firsttrip only | Takeoff only | Triplover only |
|---|---|---|---|---|
| Oneway | 422 / 0 | 200 / 166 | 200 / 137 | 422 / 0 |
| Roundtrip | 422 / 0 | 422 / 0 | 200 / 542 | 422 / 0 |
| Multicity | 422 / 0 | 200 / 65 | 200 / 181 | 422 / 0 |

Five of twelve Search scenarios succeeded. Independent Decimal calculations checked all **1,091 returned offers**: passenger selling prices, signed discount fields, preserved base/tax/AIT, and aggregate/component totals passed the configured markup calculation.

### Aggregate tax discrepancies block complete Search responses

Raw single-supplier results contained these discrepant offers:

| Supplier / itinerary | Raw offers | Offers with tax discrepancy |
|---|---:|---:|
| Triplover oneway | 263 | 20 |
| Triplover roundtrip | 972 | 99 |
| Triplover multicity | 243 | 34 |
| Firsttrip roundtrip | 503 | 2 |

Other single-supplier scenarios had no discrepancies under the audit's price checks. Inventory counts differ slightly between separate calls; those differences are not treated as defects.

In the discrepant offers, top-level and booking-component taxes do not equal passenger taxes weighted by passenger counts. Observed aggregate taxes equal one ADT + one CHD + one INF tax, omitting the second adult and CNN contribution. This describes the returned arithmetic; the supplier's internal cause is unconfirmed. Passenger-weighted totalPrice still reconciles.

Example Triplover oneway MH: ADT tax 15,534, CHD/CNN tax 13,534, INF tax 2,846. Returned aggregate tax is BDT 31,914; weighted tax for this group is BDT 60,982. Difference: BDT 29,068. Firsttrip roundtrip SQ similarly returns BDT 33,424 versus weighted BDT 64,002.

The project's existing strict pricing-coverage policy rejects the entire Search when any offer is invalid, including otherwise usable offers from other suppliers. This explains why all-three Search fails on all three itineraries. It is not an observed markup arithmetic bug. Supplier contract clarification or an explicitly agreed isolation policy is required; this audit did not rewrite taxes or silently discard invalid offers.

## Public RePrice results

| Itinerary | Supplier | Carrier | HTTP | Result |
|---|---|---|---:|---|
| Oneway | Firsttrip | BG | 200 | Passed |
| Oneway | Firsttrip | CA | 502 | `NO COMBINABLE FARES FOR CLASS USED` |
| Oneway | Takeoff | BG | 200 | Passed |
| Oneway | Takeoff | BS | 502 | `000747 NO VALID FARE FOR INPUT CRITERIA` |
| Roundtrip | Takeoff | BS | 200 | Passed |
| Roundtrip | Takeoff | TK | 200 | Passed |
| Multicity | Firsttrip | BS | 422 | `SUPPLIER_REFERENCE_MISSING` |
| Multicity | Firsttrip | TK | 200 | Passed |
| Multicity | Takeoff | BS | 422 | `SUPPLIER_REFERENCE_MISSING` |
| Multicity | Takeoff | MH | 502 | `000279 VERIFY DATE SEQUENCE IN ITINERARY` |

Five of ten public samples succeeded; all five passed independent markup checks. All ten forwarded the saved original supplier transaction/item references and the exact original flattened segment vector to the same supplier. HTTP 502 responses used `SUPPLIER_REPRICE_FAILED`. Triplover RePrice was not reached because its Search failed; no claim is made about Triplover RePrice behavior.

### Missing selected-direction support: confirmed local integration gap

The failed Takeoff MH multicity Search offer has 3, 4 and 2 alternative directions for the respective routes. The public RePrice flow flattens all alternatives into **18 segment references** and requires that exact vector from callers. These are mutually exclusive alternatives, not a single selected itinerary.

A separate read-only diagnostic reused the same original supplier transaction/item references, chose the first valid direction for each requested route/date and sent **6 segment references**. Takeoff then returned `isSuccess=true`, with one direction per route and reconciled five-passenger BDT pricing. This strongly implicates the all-alternatives payload in the earlier date-sequence failure.

This does **not** establish a working public flow: the public endpoint currently rejects subset selections. Moreover, the diagnostic response has six null segmentCodeRefs, so it would encounter the reference-validation gap below even after direction selection is supported.

Relevant implementation: `src/reprice.rs:74` compares the complete vector; `src/reprice.rs:102` forwards all original segment refs; `src/search.rs:480` flattens every direction. `docs/REPRICE_API.md:18` explicitly documents that subset/alternative selection is unimplemented. `Triploaver_API_Documentation.md:599` describes refs from selected directions in route order.

### Successful supplier RePrice responses lack segment references

Both multicity BS failures have supplier `isSuccess=true` and nonempty uniqueTransID, itemCodeRef and priceCodeRef, but all three returned segmentCodeRefs are null. The Takeoff MH six-ref diagnostic has the same issue for all six returned segments. The project's `src/reprice.rs:149` guard rejects a fare with no usable segment refs.

This is a supplier response / local contract compatibility gap. Confirm whether refreshed segment references should be returned or whether selected Search references may be reused under the supplier contract before changing validation. The audit does not assume that fallback reuse is safe.

### Remaining one-way supplier fare errors

Firsttrip CA and Takeoff BS returned upstream fare errors despite original references being preserved. Each tested offer had a single direction alternative, so the demonstrated multicity alternative issue does not explain these failures. Supplier-side investigation with private request/response evidence is needed to distinguish fare availability from other input/contract issues.

## Recommended next implementation order

1. Support selection of exactly one direction per route, validate against saved Search offers, preserve route order and persist the selected references through RePrice. Add tests for alternatives and invalid selections.
2. Resolve null RePrice segment-reference semantics with suppliers and implement the agreed handling with focused tests.
3. Resolve aggregate tax semantics with Triplover and Firsttrip; agree whether one invalid supplier offer should block all usable Search results before changing the strict policy.
4. Investigate the two one-way fare errors, then repeat the same five-passenger matrix after the changes.

## Evidence and verification

- Harness: `tests/requested_search_matrix.rs`; diagnostic: `examples/selected_direction_probe.rs`.
- Private local evidence: `.local/evidence/requested-flow-20260909-5pax/` contains incremental summary, raw supplier captures, public responses, saved original/selling offers, `analyze.py`, `analysis.json`, and `selected-direction-reprice.json`. Raw references and credentials are not included in this report or committed evidence.
- Local matrix command: run `cargo test --locked --test requested_search_matrix -- --ignored --nocapture` through `.local/run_production_reads.py`, with `LOCAL_LIVE_TEST_DATABASE_URL` pointing to the isolated `_live_search_test` database and `SEARCH_MATRIX_EVIDENCE_DIR` set to a private `.local/evidence` directory. The harness requires explicit production-read opt-in and an empty test database.
- Matrix completed in 222.16 seconds. Harness completion means all scenarios were captured, not that all API responses passed acceptance.
- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings` passed after diagnostic changes.
- RePrice coverage is ten public samples plus one direct supplier diagnostic, not every returned offer. Results describe this live snapshot and configuration. No booking or deployed runtime change was made.

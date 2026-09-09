# Requested production Search markup audit — 2026-09-08

Ran the user's exact unfiltered one-way, round-trip and multicity requests against Firsttrip, Takeoff and Triplover separately through the project's real `/api/Search` router and production adapters. All requests use 2 adults, 2 children (ages 3 and 11), 1 infant, cabinClass 1, empty carrier filters. Routes: DAC–SIN 2026-10-15; return SIN–DAC 2026-10-20; multicity DAC–SIN 2026-10-15, KUL–DAC 2026-10-20, DAC–MLE 2026-10-25.

A disposable local database used an active B2B All/All fixed BDT 500 rule and one supplier enabled per run, so failures cannot be hidden by another supplier's successful response. User's working supplier/client/markup settings were not changed. No booking, cancellation, ticketing or RePrice calls were made. Search adapters may authenticate and retry reads. Raw evidence is private under `.local/evidence/requested-matrix-20260908`; private database `matrix_20260908_live_search_test` retains original/selling snapshots.

| Request | Supplier | Upstream offers captured | Public result | Verified selling offers |
|---|---|---:|---|---:|
| oneway | firsttrip | 166 | 200 OK | 166 |
| oneway | takeoff | 136 | 200 OK | 136 |
| oneway | triplover | 251 | SUPPLIER_PRICING_COVERAGE_UNSUPPORTED | 0 |
| roundtrip | firsttrip | not captured | 503; adapter TooLarge (8 MiB cap) | 0 |
| roundtrip | takeoff | 614 | 200 OK | 614 |
| roundtrip | triplover | not captured | 503; adapter TooLarge (8 MiB cap) | 0 |
| multicity | firsttrip | 70 | 200 OK | 70 |
| multicity | takeoff | 191 | 200 OK | 191 |
| multicity | triplover | 272 | SUPPLIER_PRICING_COVERAGE_UNSUPPORTED | 0 |

## Independent price and structure audit

All 1,177 returned offers were compared against the exact same-call original snapshot using Python Decimal arithmetic, independently of the Rust pricing implementation. For every nonzero passenger type: selling = HALF_UP(original passenger total + 500, 2 decimals); discount = original base + original taxes − (rounded selling − original AIT). Multiply by supplier counts and sum; verify offer and booking component totals and component discount. Every offer's total increased by exactly BDT 2,500 for five passengers, including infant. Supplier children were represented as chd=1 and cnn=1 in the inspected sample; both types are included. Markup is not multiplied by segment, flight-option or route count.

Every downloaded successful offer matched its saved selling snapshot. Recursive comparisons verified identical key sets, array lengths and unchanged nonprice/nonreference values. Only the allowed total/discount paths and platform reference fields changed. This verifies offers, not the known unaggregated envelope summary fields; canonical cheapest-equivalent selection remains incomplete.

## Findings preventing full pass

1. Firsttrip and Triplover unfiltered round-trip reads exceeded the existing 8 MiB response limit. Adapter reported TooLarge; the public single-supplier test returned ALL_SUPPLIERS_FAILED. No complete upstream body was available, so those prices cannot be certified.
2. Triplover one-way: 251 offers received, 20 have booking component taxes inconsistent with passenger taxes multiplied by counts. Multicity: 272 offers received, 34 have the same mismatch. Current single-component validation rejects the entire Search with SUPPLIER_PRICING_COVERAGE_UNSUPPORTED; zero offers are returned, including otherwise compatible offers. These are coverage failures, not observed errors in the markup arithmetic of returned offers.
3. Example one-way Triplover discrepancy: passenger taxes are ADT 15,534 × 2 + CHD 13,534 + CNN 13,534 + INF 2,846 = 60,982, but bookingComponents[0].taxes = 31,914. Original offer/component total is 192,133.07; the passenger total aggregation agrees with it. Component semantics need investigation before changing the coverage guard or allocating values.

No limits or commercial policies were silently relaxed in this review. Next work should resolve bounded large-response handling and Triplover component representation before claiming all requested Search cases work.

## Reproduction

`tests/requested_search_matrix.rs` is an ignored, opt-in diagnostic capture harness. It requires RUN_PRODUCTION_READS=yes and an empty disposable localhost database ending `_live_search_test` through LOCAL_LIVE_TEST_DATABASE_URL. Completion of the harness does not mean all API responses passed: the matrix above records the 422/503 outcomes explicitly. Clippy passed for the diagnostic harness. Independent assertion-based audit completed for all returned offers; its private script and sanitized summary are alongside the captures.

## Firsttrip one-way example

Original aggregate 122638.33 → selling aggregate 125138.33.

| Type | Count | Original per passenger | Selling per passenger |
|---|---:|---:|---:|
| adt | 2 | 32858.12 | 33358.12 |
| chd | 1 | 25217.05 | 25717.05 |
| cnn | 1 | 23217.05 | 23717.05 |
| inf | 1 | 8487.99 | 8987.99 |

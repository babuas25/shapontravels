# Supplier fix recheck — 2026-09-09

User requested fresh verification after suppliers reported resolving all issues. Search tax discrepancies were not reproduced; missing RePrice references and supplier business errors remain. This is not full supplier acceptance.

## Scope

Production Firsttrip, Takeoff and Triplover accounts through the current local public Search/RePrice router, in two new isolated PostgreSQL databases. Existing runtime code, working database, credentials, markup rules and deployment were unchanged. The existing selected-direction implementation and strict pricing/reference validation were used without fallback or tax correction.

Same five-passenger requests as the earlier audit: 2 adults, children aged 3 and 11, 1 infant; DAC–SIN on 15 October 2026, return SIN–DAC on 20 October, and multicity DAC–SIN on 15 October / KUL–DAC on 20 October / DAC–MLE on 25 October. Test pricing was BDT 500 fixed All/All markup per passenger. Supplier calls had a 120-second timeout.

The matrix samples two offers per retained supplier after each successful Search. More Search scenarios now pass, so it produced 36 RePrices rather than the earlier 10. A focused Takeoff multicity run added one Search and two RePrices, including MH. Counts below are sampled calls, not unique itineraries or exhaustive inventory coverage.

## Results

All 12 matrix Search scenarios returned HTTP 200 with `X-Search-Partial: false`; the additional focused Search also passed.

| Itinerary | Scope | Offers | RePrice passed / sampled |
|---|---|---:|---:|
| Oneway | All three | 525 | 4/6 |
| Oneway | Firsttrip | 151 | 2/2 |
| Oneway | Takeoff | 136 | 1/2 |
| Oneway | Triplover | 261 | 1/2 |
| Roundtrip | All three | 2,096 | 6/6 |
| Roundtrip | Firsttrip | 461 | 2/2 |
| Roundtrip | Takeoff | 579 | 2/2 |
| Roundtrip | Triplover | 982 | 2/2 |
| Multicity | All three | 676 | 1/6 |
| Multicity | Firsttrip | 65 | 1/2 |
| Multicity | Takeoff | 180 | 0/2 |
| Multicity | Triplover | 431 | 0/2 |
| Multicity | Takeoff focused BS/MH | 180 | 0/2 |

Matrix: 22/36 RePrices passed. Including the focused run: **22/38 passed**, **11 HTTP 422 missing-reference failures**, **5 HTTP 502 supplier failures**.

Independent Decimal audit found zero pricing discrepancies across 6,735 raw Search offers (including repeated inventories across calls). All 6,723 retained selling offers and all 22 successful RePrices passed the existing per-passenger HALF_UP fixed-500 markup, signed discount, preserved base/tax/AIT and aggregate/component selling-total checks. The earlier aggregate tax failure is absent in these samples; this does not establish every supplier product is supported.

## Remaining failures

### Null refreshed segment references

All 11 missing-reference failures had supplier `isSuccess=true`, nonempty top-level transaction/item/price references and null references on every returned segment. Selected original Search references were forwarded correctly to the original supplier in route order; no all-alternatives payload was sent.

| Supplier | Case / carrier | Occurrences | Null / returned segment refs per call |
|---|---|---:|---:|
| Firsttrip | Multicity BS | 2 | 3/3 |
| Takeoff | Oneway BG | 1 | 1/1 |
| Takeoff | Multicity BS, including focus | 3 | 3/3 |
| Takeoff | Multicity BG | 2 | 5/5 |
| Takeoff | Multicity MH focus | 1 | 6/6 |
| Triplover | Multicity 6E | 2 | 6/6 |

The separate Takeoff-only BG oneway sample passed, so the oneway missing-reference behavior was not consistent across these calls. The focused MH call still returned six null refs after receiving exactly six selected refs. Public rejection remains `SUPPLIER_REFERENCE_MISSING`; the reference contract remains unresolved.

### Supplier failures

- Takeoff BS oneway, twice: `000747 NO VALID FARE FOR INPUT CRITERIA`.
- Triplover BG oneway, once: `FLIGHT SEGMENTS UNAVAILABLE IN THE REQUESTED CLASS`.
- Triplover TK multicity, twice: `An item with the same key has already been added. Key: DAC->IST`.

All five returned supplier business failure and public `SUPPLIER_REPRICE_FAILED`. The first two messages may relate to current fare availability; their underlying cause is not established. The Triplover duplicate-key error requires supplier investigation. Firsttrip CA oneway passed both fresh samples.

## Evidence and next step

Private raw captures, public responses, saved offer snapshots, summaries and independent audit outputs are retained in `.local/evidence/supplier-recheck-20260909-5pax/` and `.local/evidence/supplier-recheck-mh-20260909-5pax/`. Raw references and credentials are excluded from this report. Existing `tests/requested_search_matrix.rs` completed in 346.63 seconds for the matrix and 33.10 seconds for the focus run. Harness success means capture completed; it does not mean all API calls passed.

Next: have suppliers fix or clarify null refreshed segment-reference semantics and investigate the reported RePrice errors, especially Triplover's duplicate-key response. Preserve current validation until the reference contract is confirmed. Retest affected cases after correction.

No FareRules, local acceptance, Book, Cancel, PNR or ticketing verification was included in this recheck. No supplier booking/ticket mutation or deployment occurred. Other outstanding requirements remain open.

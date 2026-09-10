# User-requested BS five-route Search — 2026-09-10

Sent exactly the requested route sequence to Triplover UAT Search, using year 2026, BS preference, economy, and the preceding test's passenger mix of one adult plus one child aged 3:

| Route | Date |
|---|---|
| SPD→DAC | 22 September |
| DAC→CGP | 23 September |
| CGP→DAC | 24 September |
| DAC→CXB | 25 September |
| CXB→CGP | 26 September |

Supplier returned an empty airSearchResponses array, item2 isSuccess=false, and `No availability found for the given criteria. for USBangla`. This is a direct supplier response obtained through the read-only adapter, not a platform pricing/scope rejection. The response does not identify which leg or combination caused the lack of availability and does not prove that BS never supports multicity.

One Search only; no per-leg diagnostic searches, RePrice, passenger names, Book, Issue or database writes. Exact UAT hosts asserted; no production call. The opt-in `uat_bs_multicity_search` example has only a Search operation; formatting, targeted strict Clippy and whitespace checks pass.

Private evidence: `.local/evidence/uat-bs-five-routes-20260910T174355042523000/` (request, original response, aggregate summary; directory 0700/files 0600). No commit/push/deployment.

## User-requested four-route follow-up

Removed SPD→DAC exactly as requested and repeated one direct UAT Search: DAC→CGP 23 September, CGP→DAC 24 September, DAC→CXB 25 September, CXB→CGP 26 September 2026. Same BS preference and passenger mix. Again zero offers, isSuccess=false and `No availability found for the given criteria. for USBangla`. This does not identify the failing leg; removing SPD alone did not resolve the sampled search. No RePrice/Book/Issue. Private evidence: `.local/evidence/uat-bs-four-routes-20260910T174519241740000/`. The gated example supports explicit `UAT_BS_ROUTE_CASE=four`; default five-route behavior remains. Targeted strict Clippy/formatting pass.

## Three-route follow-up: explicit vendor limit

User then requested DAC→CGP 23 September, CGP→DAC 24 September, DAC→CXB 25 September 2026. One direct BS UAT Search, same passenger mix, returned zero offers and isSuccess=false with the explicit message: `Current vendor cannot sell more than 2 OriginDestination. for USBangla`.

This identifies a reported limit of two origin/destination pairs for the current USBangla vendor path. It is not a platform markup/validation restriction, not a guarantee that all two-route combinations are available, and not evidence that every Triplover supplier/account has this limit. Earlier four/five-route messages did not themselves reveal this reason; this later result supplies additional evidence, without proving their sole failure cause. No splitting/stitching, RePrice, Book or Issue performed.

Private evidence: `.local/evidence/uat-bs-three-routes-20260910T174647390979000/`. Gated example now supports `UAT_BS_ROUTE_CASE=three`; exact request routes inspected, targeted Clippy and formatting pass.

## All-airline follow-up: BG results available

User requested searching any airline. Repeated the same three routes/dates and 1 ADT + child age 3 with preferredCarriers/prohibitedCarriers both empty. Triplover UAT returned four BG offers: original supplier totals 24,228.00, 29,843.00, 30,720.00 and 39,496.00 in the configured BDT account currency. Each reports bookable=true and refundable=true, with all three requested directions present. These are pre-RePrice Search prices for both passengers together, without platform markup; no Hold was executed.

Supplier status array is partial: USBangla reports the two-origin/destination limit; Sabre reports invalid access token; two other entries succeed. Thus valid BG results are available despite failed upstream sources. This is direct adapter validation, not a fresh public Search/markup integration test.

Private evidence: `.local/evidence/uat-all-three-routes-20260910T174849562610000/`. Added explicit `UAT_SEARCH_AIRLINES=all` to the Search-only probe; default BS preference is unchanged. No RePrice/Book/Issue/database writes. Targeted Clippy/formatting pass.

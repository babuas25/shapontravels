# Triplover 6E FareRules investigation — 2026-09-10

## Finding

The captured error is reproducible directly through the supplier adapter, bypassing our public FareRules handler. There is no demonstrated missing request field under the project-supplied API documentation. The evidence points to Triplover's IndiGo FareRules handling or an undocumented provider-specific requirement; it does not establish the exact internal defect or responsibility conclusively.

## Request/contract audit

Project `Triploaver_API_Documentation.md`, section 4.3, documents itemCodeRef, uniqueTransID, segmentCodeRefs and optional brandedFareRefs. It specifies Search references and chosen itinerary directions. It does not document a separate fareDisplayKey input. This is the project's available documentation, not a newly confirmed supplier contract for IndiGo.

Both prior 6E requests were checked against their saved original Search offers: original transaction/item references and exactly the selected first direction per route matched, with no foreign or platform references forwarded. brandedFares was null and brandedFareRefs was correctly empty. Configured non-Search BASE_URL is used for FareRules; the adapter sends Bearer authorization.

## Fresh production read-only probe

`examples/farerules_contract_probe.rs` uses new Triplover Search results with the same five-passenger mix and dates as prior testing. It calls the supplier adapter directly, sends the four documented fields, checks FareRules before and after RePrice using the original Search references, and attempts BG as a control carrier.

| Case | FareRules before RePrice | RePrice | FareRules after RePrice |
|---|---|---|---|
| 6E DAC–SIN, 15 October 2026 | isSuccess=false; Fare display key not found for Indigo | isSuccess=true | Same FareRules error |
| 6E multicity DAC–SIN / KUL–DAC / DAC–MLE, 15/20/25 October | Same FareRules error | isSuccess=true | Same FareRules error |
| BG DAC–SIN control | isSuccess=true | Not called | Not called |

No BG multicity offer was returned for the control sample; no result is claimed for that case. All four fresh 6E FareRules attempts failed with the identical business message. Both 6E RePrices succeeded. BG's one-way FareRules success through the same adapter/configuration argues against a general host/auth failure. Calling RePrice first did not resolve the IndiGo error with the documented Search references.

No undocumented field was guessed, no opaque key was fabricated, and no Book/Cancel/Issue or local acceptance was performed. The probe does not replace production behavior. Exact private Search snapshots and supplier request/response records are in `.local/evidence/farerules-contract-20260910/`. Formatting and example Clippy passed.

## Supplier clarification still required

Ask whether IndiGo FareRules is supported with the documented Search item/segment references, and whether its fare display key should be generated/stored internally by Triplover. If a different IndiGo request contract applies, request the exact fields and a working example for the captured UTIDs. Until confirmed, report FareRules as unavailable for these samples; do not label the fares unavailable or claim missing refund/cancellation rules are known.

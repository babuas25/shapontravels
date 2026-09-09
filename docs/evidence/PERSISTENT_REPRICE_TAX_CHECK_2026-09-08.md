# Persistent Reprice tax check — 2026-09-08

Three fresh production Triplover Search calls and eleven selected Reprice calls. Read-only supplier-adapter diagnostic; no Book, PNR, Issue, Cancel, email, platform markup, database or active-setting changes.

Travel: DAC–SIN 1 November 2026; round-trip adds SIN–DAC 15 November. Cases: original five passengers (2 ADT, 1 CHD aged 11, 1 CNN aged 3, 1 INF) round-trip; isolated 1 ADT + 1 CNN round-trip and one-way. Isolated cases are diagnostic comparisons, not the original five-person booking request.

Five selected Search-mismatched offers: two corrected, three still mismatched after successful Reprice. Six consistent controls remained consistent. No eligible TK round-trip CNN mismatch was present under selection constraints; no Reprice call for that missing case. All eleven successful responses retained passenger counts and passenger taxes, and offer totals matched weighted passenger totals using independent Decimal assertions.

| Case | Search tax | Reprice tax | Weighted passenger tax | Missing CNN tax | Reprice refundable |
|---|---:|---:|---:|---:|---|
| triplover-onewaycnn-MH-mismatch | 15534.0 | 15534.0 | 29068.0 | 13534.0 | False |
| triplover-onewaycnn-TK-mismatch | 53979.0 | 53979.0 | 104958.0 | 50979.0 | True |
| triplover-roundtripcnn-MH-mismatch | 27514.0 | 27514.0 | 53028.0 | 25514.0 | False |

All three persistent cases use source `#ce25e4`, have `bookable=true` and `isPriceChanged=false`, and omit exactly the CNN passenger tax from top/component aggregates. The passenger tax values remain present. Controls under other source markers for the same airlines match, supporting a supplier source-dependent aggregation defect rather than an airline-wide defect. Supplier internal implementation remains unverified.

The original five-passenger MH and TK round-trip mismatches both corrected. Their Reprice refundable flag is false; TK changed from Search true to Reprice false. They must not be booked under the user's latest constraint.

The persistent MH examples are non-refundable and excluded from booking. The persistent TK one-way example returns refundable=true and bookable=true, but belongs to the diagnostic two-person one-way case, not the original five-person round-trip request. No booking was attempted. These flags do not independently establish actual refund conditions or resolve the prior booking's unexpected ticket-progress status.

Private evidence: `.local/evidence/persistent-tax-reprice-20260908`, including request/response pairs, Decimal audit script, and full sanitized summary. Diagnostic: `examples/persistent_tax_reprice_probe.rs`. This sample does not certify all offers or sources.

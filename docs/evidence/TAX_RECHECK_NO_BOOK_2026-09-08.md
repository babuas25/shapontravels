# Fresh tax recheck — 2026-09-08

User instruction: if the mismatch resolves before booking, do not book; report the result.

Fresh production Triplover Search and Reprice through the supplier adapter, for DAC–SIN 2026-11-01 / SIN–DAC 2026-11-15, 2 adults, children aged 3 and 11, and 1 infant. No platform markup was applied in this direct supplier diagnostic.

Search returned 877 offers. 99 had a component-tax mismatch and satisfied the diagnostic selection constraints (bookable, expected passenger counts, one component, one option per route, no branded fares). The cheapest eligible MH offer, source `#ce25e4`, was selected.

| Stage | Component tax (BDT) | Weighted passenger tax (BDT) | Supplier total (BDT) |
|---|---:|---:|---:|
| Search | 55,874.00 | 108,902.00 | 272,530.32 |
| Reprice | 108,902.00 | 108,902.00 | 272,530.32 |

Reprice succeeded, retained `bookable=true`, and returned `isPriceChanged=false`. The 53,028.00 tax discrepancy resolved before booking. Decimal assertions independently confirmed the tax aggregation and unchanged total.

Stopped as instructed. This run made no Book, PNR, NewTicket, Cancel, or email call. It does not change or establish the current status of the previous booking. No claim is made that all 99 eligible offers would resolve at Reprice.

Private request/response evidence: `.local/evidence/recheck-tax-20261101`. Read-only diagnostic: `examples/recheck_tax_probe.rs`.

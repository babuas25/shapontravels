# TK CNN pre-book Search/Reprice — 2026-09-08

User authorized fresh Search and Reprice only, requiring permission before the next flow. One fresh production Triplover Search and one Reprice completed. No Book, PNR, Issue, Cancel or email call was made. Direct supplier-adapter diagnostic; no platform markup or active settings changed.

Selected TK one-way DAC–IST–SIN, 1 November 2026, one adult and one child age 3. Intended passengers: MD ASHIF BABU and FAISL KHAN (personal details not submitted by Search/Reprice).

- TK713: DAC 1 Nov 07:25 → IST 1 Nov 13:15.
- TK208: IST 1 Nov 18:30 → SIN 2 Nov 09:55.
- Supplier total: BDT 268,772.00, unchanged at Reprice.
- Search and Reprice top/component tax: BDT 53,979.00.
- Weighted passenger tax: BDT 104,958.00.
- Missing BDT 50,979.00 equals the CNN passenger tax exactly.
- Both Search and Reprice: refundable=true, bookable=true.
- Reprice success=true; isPriceChanged=false.

Independent Decimal assertions verify the unchanged total, passenger counts and CNN omission. No Book dispatched; next flow requires user permission. Refundable/bookable are supplier flags, not independent verification of penalties or a guarantee against supplier-side ticket processing. The previous booking had an unexpected ticket-progress status despite no explicit Issue call.

Private evidence: `.local/evidence/tk-cnn-prebook-20260908`. Diagnostic: `examples/tk_cnn_prebook_probe.rs`.

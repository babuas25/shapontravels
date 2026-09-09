# Search → RePrice tax investigation — 2026-09-08

## Result

RePrice is a useful validation step, but it is **not a universal fix** for the supplier tax defect. Five fresh production Search calls yielded selected offers for 17 RePrice calls:

- 9 originally tax-mismatched offers: **5 corrected**, **2 still mismatched**, **2 supplier business failures** with no repriced body.
- 8 originally tax-consistent controls: **all 8 remained tax-consistent**.
- All 15 successful RePrice responses retained the same passenger counts and numerical passenger taxes, returned a nonempty priceCodeRef, and had offer/component totals matching weighted passenger totals.
- The 5 corrected offers changed both top-level and component aggregate taxes without changing the total fare. All 5 returned `isPriceChanged: false`.
- Both still-mismatched offers also returned `isPriceChanged: false`. That flag cannot certify aggregate tax consistency.

## Authorized scope and method

User authorized the next Search/RePrice investigation. The diagnostic uses the real supplier adapters directly, before platform markup or public routing. No database, booking, cancellation, ticketing, or FareRules calls were made. Supplier Login and bounded read retries may occur within the adapter. No pricing code or active supplier/client/markup settings changed.

Diagnostic: `examples/tax_reprice_investigation.rs`, explicitly gated by `--production-search-reprice-only`. All captured files use create-new semantics so the evidence is not overwritten. The example-specific Clippy check passed. Production business outcomes were separately checked with Python Decimal; successful tool/process exit is not treated as supplier business success.

Fresh Search references were used immediately rather than reuse prior-session captures. Requests retained cabin class 1 and empty carrier filters. One-way DAC–SIN 2026-10-15; round-trip adds SIN–DAC 2026-10-20; multicity uses DAC–SIN 2026-10-15, KUL–DAC 2026-10-20, DAC–MLE 2026-10-25.

Triplover: one-way five passengers, one-way one adult + one child age 3, round-trip five passengers, multicity five passengers. Firsttrip: round-trip five passengers. Five passengers means 2 ADT + children ages 3/11 (1 CHD + 1 CNN) + 1 INF. The two-person case is 1 ADT + 1 CNN, no CHD or INF.

For each selected carrier, one tax-mismatched offer and one consistent control were selected when available. Selection required one booking component, one direction option per requested route, and no populated branded fares. Every selected segment reference across all routes was passed unchanged and in order; request references were assertion-checked against the selected Search snapshot. Controls are distinct offers under another source marker, not necessarily identical fare products or itineraries. No qualifying TK round-trip control was available under these selection criteria; it was not called.

## Originally mismatched offers

All tax amounts below are BDT. Aggregate means both top-level taxes and the sole component's taxes; those two fields agreed in each returned response.

| Supplier | Trip / passengers | Carrier | Search aggregate tax | RePrice aggregate tax | Weighted passenger tax | Result |
|---|---|---|---:|---:|---:|---|
| firsttrip | roundtrip | SQ | 33422.0 | 63998.0 | 63998.0 | Corrected |
| triplover | multicity | MH | 56032.0 | — | 107652.0 (Search) | Supplier error; not verified |
| triplover | multicity | TK | 293102.00 | — | 581792.00 (Search) | Supplier error; not verified |
| triplover | oneway | MH | 31914.0 | 60982.0 | 60982.0 | Corrected |
| triplover | oneway | TK | 107804.0 | 212762.0 | 212762.0 | Corrected |
| triplover | onewaycnn | MH | 15534.0 | 15534.0 | 29068.0 | Still mismatched |
| triplover | onewaycnn | TK | 53979.0 | 53979.0 | 104958.0 | Still mismatched |
| triplover | roundtrip | EK | 40834.0 | 78822.0 | 78822.0 | Corrected |
| triplover | roundtrip | TK | 204654.00 | 406462.0 | 406462.0 | Corrected |

`onewaycnn` = one adult + one three-year-old child; every other row uses five passengers.

### CNN-only child case persists

Triplover MH: Search and RePrice aggregate tax **15,534** versus weighted passenger tax **29,068**. The missing **13,534** is exactly CNN's tax.

Triplover TK: Search and RePrice aggregate tax **53,979** versus weighted passenger tax **104,958**. The missing **50,979** is exactly CNN's tax.

Both Search and RePrice retain `adt=1, chd=0, cnn=1`. Passenger tax and count data are present; aggregate tax still omits CNN. In the tested five-passenger one-way/round-trip cases, which include both CHD and CNN, RePrice supplies the full weighted tax. This suggests a type-dependent aggregation path, but does not prove the supplier's internal implementation. We have not independently isolated every repeated adult/infant count in RePrice.

### Multicity supplier errors

The affected Triplover MH and TK offers did not return RePrice pricing at all (`item2.isSuccess=false`, `item1=null`):

- MH: `An item with the same key has already been added. Key: DAC->KUL`
- TK: `An item with the same key has already been added. Key: DAC->IST`

The requested multicity repeats DAC as an origin on different dates; the messages suggest a duplicate-key issue in supplier handling of repeated segment routes. The implementation/root cause remains unconfirmed. Every selected segment reference was passed as documented; no reference was fabricated or removed to bypass the error. Same-request MH/TK controls under other markers succeeded. These failures leave the affected multicity RePrice taxes unverified.

## Controls and price-change behavior

Eight controls succeeded: Firsttrip SQ round-trip; Triplover MH/TK one-way five passengers, MH/TK one-way CNN case, EK round-trip, MH/TK multicity. Their aggregate taxes match weighted passenger taxes before and after RePrice.

The EK round-trip control returned a real total-fare change from **1,036,443.18 to 1,036,100.38**, with `isPriceChanged=true`; aggregate tax remained **92,778**. All other successful selected offers had unchanged total fares. Thus a successful RePrice may correct a tax breakdown without setting the price-change flag, or may separately update the actual fare. Both numerical consistency and price-change handling are needed.

## Implications

- Continue to use RePrice before a subsequent booking decision, as documented, but do not treat RePrice success, priceCodeRef presence or `isPriceChanged=false` as proof of tax correctness.
- The observed CNN-only mismatch needs supplier correction/clarification; the multicity duplicate-key errors need a separate supplier investigation. No booking was attempted to explore either issue.
- No automatic tax rewrite, validation bypass, per-offer RePrice fallback, or public API behavior change has been implemented. This diagnostic does not make the public Search guard accept the originally mismatched offers.
- This is a selected-sample investigation (17 RePrice calls), not certification of all previously observed mismatches or all fare types. Takeoff was not repriced because the preceding tax study found no mismatched Takeoff offers.

## Evidence

Private evidence: `.local/evidence/tax-reprice-20260908` contains five Search requests/full responses, 17 selected original offers, 17 RePrice requests/responses, logs, independent Decimal analysis, sanitized summary, and SHA-256 manifest. Raw references are private (0600 captures in a 0700 directory). No supplier message was sent.

Previous investigation: [Supplier tax/pax matrix](SUPPLIER_TAX_PAX_INVESTIGATION_2026-09-08.md). Contract reference: `Triploaver_API_Documentation.md`, §4.4 RePrice.

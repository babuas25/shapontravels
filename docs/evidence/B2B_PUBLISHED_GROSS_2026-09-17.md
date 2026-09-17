# B2B published gross correction — 17 September 2026

Historical evidence: the commission basis below was superseded later the same day by [the final markup-share correction](B2B_MARKUP_COMMISSION_2026-09-17.md).

The user confirmed that B2B gross means original base fare plus taxes and that tier commission is the configured percentage of gross minus supplier net. Previously the B2B tier API used supplier net plus resolved markup as gross.

New B2B Search and RePrice now project published gross at passenger, booking-component and total levels before creating the commission snapshot. Commission and payable retain exact decimal, per-passenger rounding. Quote totals continue to reconcile with accepted-payable validation. Original supplier responses and historical accepted snapshots remain unchanged. B2C markup projection and staff supplier-cost access remain intact.

The B2B fare breakdown uses an explicit Commission column instead of displaying the gross-to-payable deduction as negative Service margin. AIT remains separately identified evidence, excluded from the user-defined published gross.

Live verification used the existing B2B account through the canonical identity bridge and only the prebooking-session, Search and pricing endpoints. For DAC → JSR on 30 September 2026, one adult, Enterprise 100% share, the current production search returned:

- Gross: BDT 5,749.00 (4,624.00 base + 1,125.00 taxes).
- Commission: BDT 485.52.
- Payable: BDT 5,263.48.
- An alternative with a higher supplier net also had gross 5,749.00, commission 353.92 and payable 5,395.08.

Validation passed: six tier tests, six foundation tests, six production-fixture tests, the full disposable PostgreSQL integration suite (46.89 seconds), strict Rust Clippy, frontend prebooking regressions, TypeScript and targeted ESLint. Integration expectations now compare equivalent numeric values independently of JSON trailing-zero representation. Tests cover multi-passenger totals, configurable shares, Search/RePrice agreement, original-reference preservation, historical snapshot stability and payable validation. Integration supplier activity is simulated only.

The running local backend was rebuilt and restarted. No real booking, ticket issue, cancellation or acceptance was submitted. Working database booking count stayed 5 and ticket issue count stayed 0. No production deployment or working schema migration occurred. Private verification metadata is in `.local/evidence/b2b-gross-20260917/live-search.json`.

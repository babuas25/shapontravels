# Public booking reference verification — 2026-09-11

User-approved format: `STR` + GDS PNR + airline PNR, no separators.

Implemented migration 0014 and owner-scoped GET `/api/bookings/by-reference/{reference}`. The reference is immutable once established; both Book and saved retrieval replies expose `X-Booking-Reference`. Admin detail/queue include `publicRef`. Missing/multiple distinct airline PNRs retain UUID retrieval. Duplicate reference pairs within one client return 409, preserving both saved supplier outcomes.

## Verification

- Ordinary all-target tests passed (40 unit, six foundation, six supplier-fixture tests; opt-in tests skipped).
- Disposable PostgreSQL suite passed in 37.77 seconds, including derivation with repeated/missing/malformed/multiple airline locators, verified PNR fallback, owner retrieval, foreign-client 404, malformed reference 404, immutable refs, and duplicate-pair 409 without repeated supplier dispatch on replay.
- Strict Clippy, Rust formatting, and Admin JavaScript syntax checks passed.
- Retained isolated `shapon_bg_multicity_uat_booking_test` backed up privately before migration to `.local/evidence/uat-bg-multicity-hold-20260910/before-public-ref.dump` (mode 600).
- Opt-in `tests/public_reference_uat.rs` applied migrations and exercised the actual application Router against the retained BG order. `STR8FE94RKECOCE` and UUID both returned HTTP 200, identical saved JSON, and matching reference header. Temporary local token removed afterward.
- This probe has zero configured supplier transports. No new Search/Book/PNR/Issue request was sent. Supplier live status/deadline remain those recorded in the prior BG report; this does not establish ticket-issue readiness.

Changes remain local; production database not migrated and no deployment performed.

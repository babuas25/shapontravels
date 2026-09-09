# Sanitized production evidence

Captured on 2026-09-08 with explicit user authorization for Login/Search/FareRules/RePrice only. See `docs/evidence/PRODUCTION_VALIDATION_2026-09-08.md` for routes, dates, sample sizes, limitations and pending decisions.

Search files retain the first three offers of each captured result; status arrays are retained. Other files retain the selected operation's response structure. Unknown/non-allowlisted nonempty strings, including all opaque references and free text, are deterministically pseudonymized by `examples/export_fixture.rs`. Numeric lexemes, counts, nulls, object fields and retained array structure are preserved. Raw captures are private and Git-ignored. These files must never be used as live supplier requests.

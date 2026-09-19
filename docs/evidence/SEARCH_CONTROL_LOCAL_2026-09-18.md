# Rust Search Control — local verification, 18 September 2026

Implemented in `Projects/shapontravels` and the existing frontend `Projects/shopontravels`, retaining the Search Control layout from `Documents/shapontravels-frontend`.

- Management and reports are Super Admin only at the navigation/page, Next mutation API, canonical business bridge and Rust handler.
- New migration 0053 stores request/supplier usage, user/supplier controls and atomic Dhaka-day counters. The selected retained localhost database was backed up before applying it. Migration succeeded; restarted port 18081 returned readiness HTTP 200. No production deployment was performed.
- `cargo clippy --locked --all-targets -- -D warnings` passed.
- `cargo test --locked` passed for the default suite; tests requiring explicit databases/providers remain opt-in.
- The new real-router integration test was separately run against an empty disposable loopback database with synthetic identity/supplier adapters. All seven non-Super-Admin roles were denied; concurrent per-user and shared supplier quotas, sub-user attribution, partial results, Dhaka date boundaries, invalid values and stale versions passed.
- Frontend TypeScript, focused ESLint and `scripts/verify-flight-search-usage-controls.mjs` passed. The latter verifies Super Admin role checks, same-origin mutation checks, input validation, version forwarding, error handling and canonical routing.
- Signed-in Chrome showed the local Search Control page with all three suppliers, date filters, summary cards and eight canonical users. Saving the existing unlimited supplier setting and unchanged enabled/unlimited user setting persisted with two audit events.
- A normal frontend Search for DAC → SIN, 30 September 2026, one adult in Economy returned 137 rendered flight options. Search Control showed one request, three supplier hits, one success, zero failures/blocks, one unique user, and the correct route/travel date under the actual actor. Each supplier's Today count was one. The recorded total request time was approximately 9.5 seconds.
- Visual inspection confirmed the existing card/table layout. The local Search Control tab was left open. No Book, Issue or Cancel was submitted.

Usage begins with this new Rust tracking; historical Supabase controls and usage were not imported. Initial limits remain unlimited. See `docs/SEARCH_CONTROLS.md` for counting, concurrency and API semantics.

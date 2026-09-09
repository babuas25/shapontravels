# Public RePrice endpoint validation — 2026-09-08

Implemented `/api/Reprice` and `/api/Reprice/accept` with migration 0007. Details and error policy: [RePrice API](../REPRICE_API.md).

- Ordinary Rust suite: 10 library tests, 3 foundation/API tests, 6 production-fixture tests passed. Opt-in supplier/database tests are run separately, not implied by the ordinary suite.
- Disposable PostgreSQL integration suite: migrations, Search/FareRules and new RePrice/acceptance checks passed, including reference tampering, markup exactly once across repeated calls, original snapshot preservation, idempotent acceptance, version supersession, foreign clients, supplier disable/epoch change, expiration, tax/currency errors and supplier business-failure redaction.
- Public real-router production smoke in isolated `reprice_endpoint_live_search_test`: Search returned 78 offers, `X-Search-Partial=false`; selected FareRules 200, RePrice 200, local acceptance 200. Search and RePrice fixed BDT 500 markup verified with BigDecimal against same-call original snapshots; returned RePrice equals its persisted selling envelope. This exercised one selected repriced offer, not every offer or every supplier's public RePrice path.
- `cargo clippy --all-targets -- -D warnings` passed.

Production supplier operations were reads only: Login, Search, FareRules and RePrice. Acceptance writes only the disposable local database. No supplier Book/Cancel/NewTicket calls; no main database migration or running-server deployment was performed.

Known CNN tax and supplier multicity defects remain subject to strict coverage/error handling. This change does not normalize supplier taxes or certify unsupported pricing shapes. Fresh supplier references are stored privately in `flight_reprices.original` and the reference map; public priceCodeRef is the platform revision UUID. Future booking must use the latest unexpired accepted snapshot and independently enforce booking/issue permissions.

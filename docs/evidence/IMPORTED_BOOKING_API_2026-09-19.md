# Imported bookings: native API read contract

All three import sources are covered: Airline (`IMP_EXP`), Supplier API (`SUPPLIER_API`) and Manual (`MANUAL`). They remain in the authoritative import store and are projected through existing API read routes; no duplicate native booking or supplier mutation is created.

## Implemented

- Booking retrieval by UUID and canonical `STR…` reference, saved ticket receipt, ticket report by UUID/reference/transaction, PNR, reconciliation and booking pricing.
- Native `BookingReceiptResponse`, `TicketReceiptResponse`, `PnrResponse`, `TicketReport` and `PricingSnapshot` contracts. Import-success responses include native receipt fields alongside existing portal metadata.
- Migration 0058 backfills stable transaction/item/price/ticket UUIDs and creates them transactionally for every future import. Stored identifiers cannot be updated or deleted.
- Ownership comes from the canonical agency owner linked to the authenticated API client. Existing API eligibility and endpoint permissions still apply. Import administration remains Super Admin only.
- Customer responses retain accepted payable and published gross separately. Supplier invoice costs, raw supplier evidence and pricing rule identifiers are excluded.
- Import PNR/reconcile/report endpoints return saved evidence, identified by `X-Evidence-Source: saved-import`; they do not perform a live supplier lookup. Imported identifiers cannot authorize native supplier issuance/cancellation.
- Incomplete historical fares are not synthesized. Booking/ticket can have nullable `flightInfo`; pricing/report returns 409 where required evidence is absent. Legacy single-type fares preserve the actual captured payable and expose unknown historical tier as null. Ticket evidence must meet the native 10–16 digit format.

## Verification

- `cargo test --locked --lib`: 93 passed, 2 explicitly ignored environment-dependent tests.
- Disposable database integration: `cargo test --locked --test dashboard_sections -- --ignored` passed using synthetic data only.
- `node scripts/verify-import-api-contract.cjs .local/import-api-contract/responses.json`: 34 responses validated against the unchanged native schemas in `src/client_schemas.json`, including all three sources and import-success receipts.
- Integration verifies native endpoints, reference consistency/replay, held versus issued behavior, agency/sub-user ownership, cross-agency denial for every source, permission denial, mismatched PNR references, saved pricing despite later tier changes, no private rule/cost leakage, and unchanged wallet operation count and balance after reads.
- `cargo clippy --locked --lib --test dashboard_sections -- -D warnings` and binary/local runtime builds passed.
- Local retained database was backed up privately before migration 0058. Every existing import has an API reference row. Wallet account, operation and ledger hashes match before and after migration/restart.
- The existing reported ticket has backfilled API references and a canonical API-client owner mapping. Its gross and captured payable were preserved. No extra debit or historic repricing was performed.

Local implementation only; no production deployment or real account API-access changes were made. Local health readiness passed after restart. Supplier systems were not called by these tests.

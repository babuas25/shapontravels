# Flight Search Control

Implemented in Rust and the sibling `shopontravels` frontend at `/dashboard/search-control`, using the existing reference frontend layout. The page, navigation, Next API, canonical business bridge and Rust handler all restrict management/report access to **Super Admin**. Search users continue to search under their existing roles; the saved policies apply to them.

Apply migration `0053_search_controls.sql` through the normal migrator before serving this build. It creates empty usage/counter tables and unlimited supplier defaults; it does not import legacy Supabase controls or reconstruct old usage from retained offers. Reporting starts when this build handles Search.

## Behavior

- One authenticated, valid Rust `/api/Search` request records one request. Authentication failures and invalid bodies are not usage events. Admission rejection is recorded as blocked.
- Each supplier Search dispatch reserves one hit for the actual actor and one for the supplier. Failed/timed-out dispatches consume their reservation. Login, internal transport retries, FareRules and RePrice are not separate hits. Reservations are conservative: cancellation/crash after reservation does not refund quota; unfinished requests remain pending rather than being reported as completed successes/failures.
- Canonical search sessions attribute agency sub-user searches to the sub-user, even though their pricing client belongs to the owner. Machine API calls use their linked owner; unlinked clients are grouped in the report's anonymous row and remain separately keyed for counters.
- A user daily limit is shared across all suppliers. A supplier daily limit is shared across users and API clients. Actor advisory locks and supplier row locks serialize reservations across processes. No lock spans supplier network I/O.
- A disabled user gets `403 USER_SEARCH_DISABLED`. Exhausted user/supplier quotas get `429 USER_DAILY_LIMIT_REACHED` or `SUPPLIER_DAILY_LIMIT_REACHED` when no dispatch can run. Other suppliers can still return partial results when one supplier is blocked. Existing in-flight requests are not recalled by a subsequent control change.
- Daily budgets use the current database day in `Asia/Dhaka`, computed after acquiring locks. Limits reset by using a new day key at midnight; no scheduled reset is required.
- Supplier limits accept `null` (unlimited), `0` (blocked), or 1–1,000,000. User limits accept `null` or 1–1,000,000, with a separate enable/disable flag.
- Reports use inclusive Dhaka calendar dates, up to 367 days. Today counters always show today, independently of the selected historical range. Routes include travel dates. Request outcomes and supplier outcomes are separate: a successful partial request can include a blocked/failed supplier.
- Control writes require the report's version. A new user control starts at version 0; supplier defaults start at 1. Conflicts return 409. Each successful change and its audit event commit in the same transaction. Failed reads never render writable default controls.

## API

The canonical frontend uses `business/execute` with `POST /admin/search-control`. Rust derives the actor from the verified session and checks Super Admin again in the domain handler. The endpoint is also in authenticated Admin OpenAPI, excluded from the commercial contract.

```json
{"action":"report","from":"2026-09-18","to":"2026-09-18"}
{"action":"supplier","supplier":"triplover","daily_limit":5000,"expected_version":1}
{"action":"user","subject":"user_example","search_enabled":true,"daily_limit":100,"expected_version":0}
```

The Next `/api/search-control` mutation endpoint checks same-origin requests, input limits and Super Admin authority, then maps camel-case form fields to this contract. No legacy Server Actions or Supabase reads/writes are used by this screen.

## Verification

`tests/search_controls.rs` uses a new empty database ending `_search_control_test`, fake identity and supplier adapters, and the actual routers. It covers all seven non-Super-Admin roles, date boundaries, stale versions, null/zero limits, per-user and cross-user concurrent supplier quotas, partial results and agency sub-user attribution. Run with `SEARCH_CONTROL_TEST_DATABASE_URL=... cargo test --test search_controls -- --ignored`.

Frontend: `node scripts/verify-flight-search-usage-controls.mjs`, `npm run typecheck`, and ESLint on the modified frontend files. No live supplier writes are needed for these checks.

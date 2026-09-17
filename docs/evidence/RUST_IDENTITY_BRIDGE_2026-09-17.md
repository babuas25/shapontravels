# Rust identity bridge/bootstrap — Phase 2 local evidence

Scope: Phase 2 of [the requirements plan](../RUST_IDENTITY_REQUIREMENTS_PLAN.md). Phase 2 is complete as staged code; Phase 3 is next. This is not a live account-management migration or a claim that existing Supabase account creation/deletion works.

## Current implementation verified first

Read the plan, linked Phase 1 evidence, migration 0034, identity policy/tests, Rust router/authentication and the Next API Management/session/security entry points. Both repositories contained substantial prior uncommitted work; it was preserved. No reset, stash, checkout, commit, real account deletion or data cleanup was performed.

Before implementation, all three existing identity unit tests passed. The Phase 1 PostgreSQL foundation integration test passed on a new `phase2_baseline_identity_test` database, including last-admin concurrency, immutable keys/audit, membership integrity and rollback.

## Delivered

- Rust migration 0035: immutable one-time bootstrap record, with a staged-only authority marker. No users are seeded by migrations.
- `src/identity/api.rs`: dedicated bridge/operator authentication, strict bounded requests, current registry/session reads, explicit serialized bootstrap, idempotent least-privilege onboarding, transactional audit and retained-reference blockers. Default route mode is disabled.
- `src/identity/provider.rs`: strict read-only Clerk provider interface plus production GET adapter; tests use an in-memory fake. Ban/lock/deletion/errors fail closed; provider metadata grants no role.
- Rust router/main/OpenAPI registration; identity routes excluded from public commercial API docs.
- Next `lib/rust-api/transport.ts`: actor-independent transport extracted from API Management without changing existing exports or authorization behavior.
- Next `lib/identity/{server,transport}.ts`: server-only staged bridge, current Clerk session validation, canonical response parsing and sanitized typed errors. No existing dashboard/actions were switched.
- Offline Next regression `scripts/verify-rust-identity.mjs` and `verify:rust-identity` package command.
- Loopback-only explicit operator command `scripts/identity-bootstrap.mjs`; no automatic environment-file loading and no secrets on the command line/output. It was syntax-checked, not run against a provider account.
- [API/setup contract](../PORTAL_IDENTITY_API.md), example configuration and updated phase log.

## Isolation

Used the existing stopped **disposable** cluster `.local/identity-review-db`, listening on `127.0.0.1:55451`, and created new empty databases:

- `phase2_baseline_identity_test` — existing Phase 1 test before changes.
- `phase2_routes_01_identity_test` — initial real-router Phase 2 run.
- `phase2_routes_02_identity_test` — final router run, including bootstrap audit-failure rollback and typed body errors.

Both integration suites validate loopback host, `_identity_test` suffix and an empty public schema before migration. Each run uses a newly created database; evidence databases were retained, not reset. The disposable cluster was stopped after verification.

No `.env`/`.env.local` secret, deployed/runtime flag, existing portal database, Clerk identity, provider invitation, financial row, supplier action or ticket was modified. Synthetic local rows were used for suspension/tombstone/rollback tests. Provider verification made no real Clerk requests.

## Verification results

All passed:

1. Existing identity unit tests and pre-change PostgreSQL foundation integration.
2. Full `cargo test --locked`: **80 passed** (67 library, 6 foundation, 1 disabled/unavailable identity route, 6 fixture). Opt-in integration/UAT suites remained ignored in this ordinary run.
3. Explicit real-router identity PostgreSQL integration: 1 passed on the final fresh database.
4. `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`.
5. Next offline identity bridge regression, existing API Management/security regressions, TypeScript `tsc --noEmit`, and targeted ESLint.
6. Bootstrap command syntax check and whitespace checks in both repositories.

Commands (Cargo required its absolute path on this shell):

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked identity::tests --lib
IDENTITY_TEST_DATABASE_URL=postgres://ashifbabu@127.0.0.1:55451/phase2_baseline_identity_test \
  /Users/ashifbabu/.cargo/bin/cargo test --locked --test identity -- --ignored --nocapture
/Users/ashifbabu/.cargo/bin/cargo test --locked
IDENTITY_API_TEST_DATABASE_URL=postgres://ashifbabu@127.0.0.1:55451/phase2_routes_02_identity_test \
  /Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_api -- --ignored --nocapture
/Users/ashifbabu/.cargo/bin/cargo fmt --check
/Users/ashifbabu/.cargo/bin/cargo clippy --locked --all-targets -- -D warnings
node --check scripts/identity-bootstrap.mjs
git diff --check
```

Run the Next commands from `/Users/ashifbabu/Projects/shopontravels`:

```sh
node scripts/verify-rust-identity.mjs
node scripts/verify-api-management.mjs
node scripts/verify-account-security-lock.mjs
./node_modules/.bin/tsc --noEmit
./node_modules/.bin/eslint lib/api-management/server.ts lib/rust-api/transport.ts lib/identity/server.ts lib/identity/transport.ts
git diff --check
```

Do not rerun the database commands against the named evidence databases: create new empty test databases first.

## Proven boundaries

- Disabled routes and database outages deny access; unauthenticated, machine, ordinary admin, wrong bridge and wrong operator capabilities are rejected before provider reads.
- The bridge cannot bootstrap. Bootstrap uses its configured operator identity, verifies the selected provider user, and never promotes a first login automatically.
- Concurrent bootstrap produces exactly one Super Admin; the marker and audit cannot be overwritten. Failure inserting the audit rolls back the user and marker.
- Concurrent/repeated onboarding produces one customer identity and one audit, no agency, wallet or API client. Session reads do not provision users.
- Forged role/owner/agency/password/operator fields, invalid subject paths and oversized bodies are rejected.
- Provider missing/banned/locked/mismatched/outage states fail closed. Unverified primary email is not treated as verified contact data; metadata roles are ignored.
- Existing suspended/deleted identities remain denied after onboarding replay. Same-email/new-Clerk-ID registration has a different immutable UUID.
- Known retained user-wallet references block new mapping. Onboarding audit failure rolls back user insertion; subsequent retry succeeds. Transaction contention returns a distinct busy code.
- Next verifies current session subject/status/expiry and rejects signed-out, revoked, ended, expired, mismatched and missing provider sessions before Rust calls. Function arguments cannot inject another subject; metadata is not forwarded.
- Next rejects invalid backend modes/credentials/roles/subjects and preserves typed unavailable errors without falling back to Supabase/Clerk authority.
- Existing API Management route permissions, audit failure behavior and account-security regressions pass after the transport split.

## Remaining limitations / next phase

- **Phase 3 is pending:** existing `getAccountSession`, dashboard preview behavior, middleware, account actions and API/business consumers still use their previous authority contracts. The new adapter is not wired into live authorization.
- Phase 4 must add durable provider mutation operations, invitations/acceptance, effects/inbox, recovery and full lifecycle/dependency guards. Onboarding here only registers an already authenticated provider subject as an applicant/customer; it never creates a Clerk account.
- Retained agency-code-only owners require explicit inventory/matching in later gates. No email/name mapping or automatic migration was attempted.
- Real Clerk HTTP contract/provider environment, browser account journeys, full opt-in wallet/booking DB suites and deployment readiness were not exercised. No UI was changed in this phase; browser journeys belong to later integration/UI gates.
- No live cutover, user invitation, destructive workflow, bootstrap of a real operator or actual target-environment activation was performed.

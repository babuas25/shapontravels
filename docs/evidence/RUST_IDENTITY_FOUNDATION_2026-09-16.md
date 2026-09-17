# Rust identity foundation — local verification

Scope: Phase 1 of [the identity requirements plan](../RUST_IDENTITY_REQUIREMENTS_PLAN.md). This records a staged foundation, not a completed account-management migration.

## Changes

- `migrations/0034_portal_identity_foundation.sql`: additive empty registry, agency, membership and append-only audit tables. Deferred foreign keys permit atomic user/agency/owner membership creation and prevent orphaning an agency. Active B2B identities require membership at commit.
- Immutable user UUID/Clerk mapping and agency UUID/code; users are tombstoned and agencies archived instead of cascading deletion. Same email is permitted on separate identities and never merges ownership.
- Database-controlled versions/timestamps and membership authorization invalidation.
- `src/identity.rs`: strict roles/statuses; global account policy and the existing B2B-owned-sub-user exception; serialized transaction helper, last-active-Super-Admin guard and allowlisted audit metadata.
- `tests/identity.rs`: disposable PostgreSQL transaction/integrity regression. No identity routes registered and no Clerk adapter yet.

## Database isolation

- Created a new cluster at `.local/identity-review-db`, listening only on `127.0.0.1:55451`.
- New empty database: `foundation_01_identity_test`.
- Test validates loopback host, database suffix and empty public schema before running all migrations.
- No existing portal database, real account, supplier, wallet, environment secret or runtime authority setting was changed.
- The isolated cluster was stopped after verification; its database is retained as local evidence.

## Verification

Results: 3 targeted identity unit tests passed; the explicit PostgreSQL integration test passed. The ordinary full Rust run passed 77 tests (65 library, 6 foundation, 6 fixture); opt-in integration tests remained ignored in that ordinary run. Formatting, all-target Clippy with warnings denied and diff whitespace checks passed. The existing frontend offline account-security regression also passed.

```sh
cargo test --locked identity::tests --lib
IDENTITY_TEST_DATABASE_URL=postgres://ashifbabu@127.0.0.1:55451/foundation_01_identity_test \
  cargo test --locked --test identity -- --ignored --nocapture
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
git diff --check
```

The database command requires a **new empty database per run**; the name above is the retained evidence database, not a reusable test fixture to reset. The current shell required `/Users/ashifbabu/.cargo/bin/` for Rust commands because Cargo was not on PATH.

Proven by the new tests:

1. Strict role serialization and management/self/target/status boundaries.
2. B2B owner scope cannot reach another agency, an administrator or a peer owner.
3. Fresh migrations and schema readiness; no automatic user provisioning.
4. Atomic owner/agency/member creation; orphan activation and owner removal/demotion refused.
5. Duplicate agency ownership/current membership and stable-key mutation refused.
6. Membership edits advance authorization version; display edits do not.
7. Concurrent demotions preserve one active Super Admin; suspending the survivor is refused.
8. Lock contention returns `IDENTITY_OPERATION_BUSY`; rollback releases the lock.
9. Failed audit insertion rolls back the local edit; audit update/delete/truncate refused.
10. Tombstone resurrection/deletion and Clerk-ID reuse refused; same-email replacement has a different immutable identity.

## Remaining boundaries

- These policies are library helpers, not yet wired to application endpoints. Mutations must use the shared transaction helper and derive actors from the database in later phases.
- Provider operations, idempotency/outbox/inbox, bootstrap, role/session integration, profile/application parity and business dependency checks are still pending.
- No browser account-create/delete test was run because the new identity API/UI is not implemented yet. The existing Supabase dependency is still active.
- Existing real-DB wallet/booking suites that are opt-in are not counted as executed by plain `cargo test`. Phase 6/8 will rerun the appropriate business integration suites after wiring identity authority.

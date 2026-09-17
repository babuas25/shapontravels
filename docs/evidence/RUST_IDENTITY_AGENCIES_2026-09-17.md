# Phase 4 agency lifecycle evidence — 2026-09-17

Status: agency provisioning/reactivation slice verified; **Phase 4 is still in progress**. Clerk account creation/invitations/deletion, production writer/worker and durable inbox remain pending. Existing uncommitted changes were preserved. Next preview/legacy authority gates and real environment files were not changed. No real account/provider write, invitation, live migration or cutover occurred.

## Changes

- Migration 0037 extends operation actions; adds a non-recycling six-digit agency code allocator, immutable agency-wallet binding with database identity validation, and immutable agency operation result snapshots.
- `src/identity/agencies.rs` implements fresh customer-to-B2B provisioning and explicit owner/agency reactivation inside the existing authority transaction. It reuses stored owner/membership data and blocks known retained subject references.
- Provisioning performs strict fresh wallet inserts: one BDT account with zero available/held balance and no ledger, credit or API-client grant. The wallet owner's agency key is the newly allocated immutable display code, bound to the internal agency UUID. It never adopts an existing financial owner.
- Reactivation preserves agency/wallet identity and individual sub-user statuses. It invalidates member authority and retains disabled credentials/frozen wallet state. Archived agencies cannot be revived.
- Existing operation replay/version/scope/audit/outbox semantics apply. Provisioning and reactivation share the original role/access action limits. The HTTP bridge verifies both current actor and target provider users outside the transaction; current database authority/versions are rechecked inside it.
- `OperationView.agency` holds the original agency result, including version/status; old operation results do not change when the current agency later changes. Agency suspensions now include this snapshot and agency audit target as well.
- Shared synthetic helpers are in `tests/identity_support/mod.rs`, used by both operation and agency integration tests.

## Executed checks

Fresh-install PostgreSQL checks ran in newly created empty databases on the isolated cluster at `127.0.0.1:55451`, with migrations through 0037. A separate additive-upgrade check used a newly created clone of the retained synthetic migration-0036 evidence database. Test helpers reject remote hosts, wrong database suffixes and nonempty public schemas. No existing database was reset. Databases remain as evidence; the isolated cluster was stopped afterward.

| Check | Result |
| --- | --- |
| `cargo check --locked` | Passed |
| `cargo test --locked --quiet` | 82 ordinary tests passed; live/provider/browser/disposable-DB opt-in checks remain ignored by default |
| Explicit agency and operation lifecycle tests | 4 passed: agency strict-input test, full agency PostgreSQL matrix, disabled operation/schema test and prior full recovery PostgreSQL matrix |
| Final agency database | `phase4_agencies_02_identity_test` |
| Role/access/recovery regression database | `phase4_agency_ops_02_identity_test` |
| Foundation integrity regression | Passed in `phase4_agency_foundation_identity_test` |
| Bootstrap/bridge/session regression | Passed in `phase4_agency_routes_identity_test` |
| Additive upgrade from migration 0036 | Passed in `phase4_agency_upgrade_identity_test`, cloned from `phase4_operations_03_identity_test`; hashes of 11 existing identity/operation/audit/wallet tables unchanged |
| `cargo fmt --check`, `git diff --check` | Passed |

The first agency integration run passed its PostgreSQL scenarios but its strict-schema test caught Serde accepting extra fields on a unit enum variant. The command was changed to a strict empty-struct variant; the final run rejects agency-code/wallet/balance overrides. The earlier `phase4_agencies_01_identity_test` database is retained; it was not reset. `phase4_agency_ops_01_identity_test` remained unused when the first test executable reported failure.

Verified scenarios:

1. Current manager scope, self-change, owner/sub denial, eligible customer state, inappropriate staff/admin target, suspended target, banned/missing/unavailable provider target and retained client ownership guards.
2. Code collisions with an existing retained funded wallet and a registry agency are skipped. The synthetic retained ledger content stays byte-for-byte equivalent as JSON; no link/balance is adopted.
3. Injected audit failure rolls back user role/version, agency, membership, fresh wallet/binding, operation/result/effects. Sequence values consumed during failure are intentionally not recycled.
4. Concurrent exact replay returns the same operation and one new agency/wallet. Distinct concurrent operation IDs with the same expected user version produce one success and one conflict. Changed payload conflicts; stale versions cannot create duplicates.
5. Fresh agency membership resolves through the session endpoint as the stored B2B owner. Its wallet has zero available/held balance and version zero; no new machine client or ledger posting is created.
6. Agency-wallet identity mismatch and changes/deletion of immutable bindings/results are rejected by PostgreSQL.
7. Synthetic provider delivery completes the operation using the existing ordered/fenced outbox. Previous role/status failure, timeout-unknown, lease, stale-worker and reconciliation tests still pass with migration 0037.
8. Suspension produces an agency version snapshot. Reactivation rejects stale/invalid agency versions, wrong scope and wrong target type. Audit failure rolls back owner, agency and member authorization changes.
9. Concurrent reactivation replays once, keeps the same agency/wallet/account, restores effective access for active members and retains individually suspended sub-users. Frozen wallet and disabled owner/sub-client access remain unchanged; provider effects are queued for the affected subjects.
10. Archived agency reactivation is denied. Shared role/access rate buckets prevent alternate-command allowance bypass. A 25-collision batch returns a typed conflict without authority mutation and can retry safely; exhausted six-digit code space fails without a partial account.

To reproduce lifecycle tests, create fresh loopback databases with names ending in `_identity_test`; set `IDENTITY_AGENCIES_TEST_DATABASE_URL` and `IDENTITY_OPERATIONS_TEST_DATABASE_URL` to those empty databases, then run:

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_agencies --test identity_operations -- --include-ignored --skip additive_agency_upgrade_preserves_existing_evidence --nocapture
```

The separate upgrade test requires a **new clone** of synthetic migration-0036 evidence, with `agency_upgrade` in its name and `_identity_test` suffix. Set `IDENTITY_AGENCY_UPGRADE_TEST_DATABASE_URL` to the clone, then run only `additive_agency_upgrade_preserves_existing_evidence -- --ignored`. It checks the schema version and snapshots all 11 existing identity/operation/audit/wallet tables before applying the additive migration, and asserts no inferred bindings/results. Do not point it at the original evidence database. Do not load production environment files or reset evidence databases.

## Continuation boundary

This is provisioning for an **already registered and verified** identity, not provider account creation or B2B application approval. Only eligible customers are accepted; role transfers, ownership transfer and historical identity/financial matching need explicit later workflows. The Next management UI is not connected to these commands. The production Clerk adapter still performs reads only, so staged operations remain pending until a writer/worker is implemented; all delivery in tests was synthetic.

Continue Phase 4 with provider account creation (durable prepared intent and password-safe unknown-outcome recovery), invitation lifecycle, deletion/dependency dry run, production provider adapter/worker and signed durable inbox. Application decisions/profile transfer, business integration, browser parity and live activation remain later gates. No claim is made that live account create/delete now works without Supabase.

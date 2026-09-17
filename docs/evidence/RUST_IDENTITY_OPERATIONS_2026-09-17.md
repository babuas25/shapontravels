# Phase 4 role/access operations and recovery evidence — 2026-09-17

Status: first Phase 4 slice verified, **Phase 4 remains in progress**. All provider responses/writes below are synthetic. No live activation, real Clerk write, invitation, account deletion or financial reset occurred. Existing uncommitted work was preserved; this slice changes only Rust identity code/tests/migration and documentation. Next preview gates and default legacy authority remain in place.

## Implemented slice

- Migration `0036_portal_identity_operations.sql`: immutable operation request/result snapshots; ordered provider effects; durable fenced attempts; retention and transition guards.
- `src/identity/operations.rs`: strict role/access requests and scoped operation queries, current database authorization, expected versions, exact replay/changed-payload conflicts, self/last-admin checks, agency dependency guards and shared per-actor limits (role 30/hour, access 40/hour).
- Atomic local authority/audit/outbox/API revocation. Owner suspension also suspends the agency and invalidates sub-user authority. Financial rows/links survive unchanged. Activation never auto-enables an API client.
- `src/identity/effects.rs`: injectable provider interface, pre-dispatch durable attempt, 30-second fenced lease, 10-second provider deadline, per-target ordering, superseded old metadata, five-attempt cap with 30-second backoff only for proven-not-sent delivery, read-only reconciliation for uncertainty or lease expiry.
- Staged bridge-only POST operations/query routes with strict bodies, current provider actor lookup, safe errors and no-store responses. Results distinguish local commit from pending/unknown provider synchronization. Effect detail is capped at 100, with total/truncation; recovery scans are capped at 25 per invocation.
- Role/status audit details include prior/next status, role and row versions. Claim/result audit failures roll back database state; no network call is made before a successful claim commit.

## Executed verification

The isolated cluster `.local/identity-review-db` listened only on `127.0.0.1:55451`. Every database below was newly created with an empty public schema and migrated through 0036; none was reset/reused. The cluster was stopped after verification; evidence databases are retained.

| Check | Result |
| --- | --- |
| `cargo check --locked` | Passed after implementation |
| `cargo test --locked --quiet` | 81 ordinary tests passed; provider/live/browser/disposable-DB opt-in tests remain ignored by default |
| `cargo fmt --check` | Passed |
| `git diff --check` | Passed |
| `tests/identity_operations.rs --include-ignored` | 2 passed, including the full PostgreSQL failure/replay matrix, latest database `phase4_operations_03_identity_test` |
| `tests/identity.rs --ignored` | Foundation integrity regression passed in `phase4_foundation_identity_test` |
| `tests/identity_api.rs --ignored` | Bootstrap/bridge/session regression passed in `phase4_routes_identity_test` |

Earlier operation runs in `phase4_operations_01_identity_test` and `phase4_operations_02_identity_test` also passed; the final run adds status audit assertions. No real environment file was loaded/modified for these commands. PostgreSQL credentials were local OS-user authentication, not provider secrets.

Operation/recovery integration assertions include:

1. Disabled routes, wrong token families, strict action/schema/password rejection, body-size error and banned actor rejection.
2. Current role/scope guards: self-change, unauthorized Super Admin target/grant, customer actor, own-agency sub access, cross-agency denial, forbidden owner grants, membership/provisioning/reactivation dependencies and tombstone revival denial.
3. Concurrent identical request replay creates exactly one operation, audit event and effect pair; changed payload conflicts; stale versions conflict; operation lookup rejects unrelated callers and omits claim tokens.
4. Injected local audit failure rolls back role/version, operation/effects and linked-token revocation. Successful mutation disables both managed and staff-linked client/credentials and removes machine/prebooking sessions. Synthetic wallet ledger content and immutable client link remain unchanged.
5. Concurrent worker claims produce one claim for the subject. Same-result completion replay is idempotent; operation/effect/attempt retention and immutable inputs are enforced.
6. Provider fakes verify attempt visibility and acquire the same authority transaction lock during the call, proving the attempt committed and the provider call holds no authority transaction.
7. Audit failure before claim prevents provider dispatch; audit failure after a claim leaves it dispatching and blocks resend until result persistence/recovery.
8. Unknown dispatch remains visible and is never automatically resent. Verified observation completes it; stale reconciliation fences are rejected. Proven-not-sent retries honor delay and the five-dispatch cap; absence/not-sent observations do not authorize another write.
9. Expired dispatch and expired reconciliation leases require recovery. Older worker/fence results cannot finalize a newer attempt, and recovery makes no provider write.
10. Newer local authorization supersedes pending older metadata; canonical role remains the newer Rust value. Owner suspension invalidates sub authority and revokes linked access in the same transaction.
11. Exact per-actor action limits deny without local changes. A 101-sub-user agency produces 103 persisted effects but returns only 100 summaries with explicit truncation. Simultaneous cross-suspension of two Super Admins leaves exactly one active Super Admin.

Reproduction uses a **new** database name ending in `_identity_test` each time. Set `IDENTITY_OPERATIONS_TEST_DATABASE_URL` to that empty loopback database and run:

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_operations -- --include-ignored --nocapture
```

The test itself rejects a remote host, wrong suffix or nonempty public schema. Never reset an evidence/real database to satisfy it.

## Remaining Phase 4 work and boundaries

- No production implementation of `EffectProvider`, dispatch scheduler or provider write is connected. Staged HTTP operations intentionally remain `pending_effects` until that implementation exists. No browser force-complete/retry endpoint is exposed.
- Create/provisioning, invitation/revoke/acceptance, deletion/dependency dry run, agency reactivation, signed durable event inbox and out-of-order provider-event handling remain unimplemented.
- Full recovery UI, cursor-based queue browsing (including truncated effect details), denied-operation audit coverage, metrics and deployment/operator recovery workflow remain pending.
- The existing Next preview still blocks mutations/business routes. No new Next operation transport or management UI was enabled. No browser lifecycle journey or live account create/delete fix is claimed.
- Synthetic tests validate local ordering, recovery and fencing. They do not prove Clerk write/reconciliation semantics; those must be implemented and tested against the official provider contract before wiring a writer. Metadata is a mirror and can never become the Rust role authority.
- Phases 5–9, live migration/cutover and actual account-management Supabase removal are still pending. Continue with the remaining Phase 4 workflows, preserving current changes and evidence.

# Phase 4 durable account-create evidence — 2026-09-17

Status: staged create orchestration and local finalization verified with synthetic adapters. **Phase 4 remains in progress.** Production Clerk write/reconciliation adapters, invitations/deletion, signed durable inbox, mail effects and recovery UI are not connected. This does not fix or activate the live account-create path yet.

Existing uncommitted work, real accounts/data, environment secrets and Next preview gates were preserved. No real provider account, invitation or financial transaction was created. All provider writes in these checks were in-memory fakes.

## Implemented

- Migration 0038 adds immutable password-free create intents, email in-flight reservations, fenced durable attempts, transition/retention guards and the ordinary `create_account` operation action.
- `src/identity/creates.rs` separates normalized persisted intent from transient password input; prepares/audits before provider dispatch; handles confirmed/not-sent/unknown outcomes; recovers expired dispatch/reconciliation claims; allows cancellation only on known-safe prepared state; and commits canonical identity/agency/membership/wallet/effects atomically after provider confirmation.
- API routes: staged `/creates/prepare`, `/dispatch`, `/query`, `/finalize`, `/cancel`. Production runtime has no writer and returns `IDENTITY_PROVIDER_WRITE_DISABLED` on dispatch. Only explicit adapter injection enables synthetic tests. No browser force-complete or user-supplied provider-evidence endpoint exists.
- Current actor role/access/grant and original-creator-or-Super-Admin recovery scope are rechecked. Sub-user agency/owner state, expected agency version and 100-member capacity (including pending reservations) are checked before dispatch and local commit.
- Provider identity is re-read before local commit; existing mappings/retained financial references are rejected. B2B gets a fresh agency/zero BDT wallet, customer stays onboarding, and sub-user membership uses its authorized stored agency.
- Known pending subject/verified-email reservations block automatic onboarding as a denial guard. No email match can attach an existing identity or financial owner. An uncorrelated/no-verified-email onboarding race fails closed at finalization and remains an explicit production-wiring/recovery concern.
- Existing role/access operation IDs cannot consume a prepared create's UUID. Completed create IDs also identify the ordinary durable provider-effects operation.

## Verification executed

PostgreSQL ran only on the isolated cluster at `127.0.0.1:55451`. Fresh-install tests rejected nonempty schemas, remote hosts and wrong database suffixes. The upgrade test used a new clone of retained synthetic evidence, not the original. No database was reset; evidence databases were retained and the cluster stopped after verification.

| Check | Result |
| --- | --- |
| `cargo check --locked` | Passed |
| `cargo test --locked --quiet` | 83 ordinary tests passed; opt-in live/browser/database checks remain ignored by default |
| Explicit `identity_creates` suite, including ignored cases | 3 passed: password/input separation, full create/recovery PostgreSQL matrix, additive-upgrade preservation |
| Latest create matrix database | `phase4_creates_03_identity_test` |
| Upgrade clone | `phase4_create_upgrade_identity_test`, cloned from `phase4_agencies_02_identity_test` at migration 0037 |
| Upgrade integrity | 15 preexisting identity/agency/audit/financial/control/rate table fingerprints unchanged; no creates/attempts fabricated |
| Foundation regression | Passed in `phase4_create_foundation_identity_test` |
| Bootstrap/session/onboarding regression | Passed in `phase4_create_routes_identity_test` |
| Role/access/effect recovery regression | Passed in `phase4_create_ops_identity_test` |
| Agency lifecycle regression | Passed in `phase4_create_agencies_identity_test` |
| `cargo fmt --check`, `git diff --check` | Passed |

Earlier create matrix runs in `phase4_creates_01_identity_test` and `phase4_creates_02_identity_test` passed and remain retained. The final matrix adds re-registration/retained-financial-reference assertions; the new upgrade test covers migration 0038 against existing synthetic data.

The create tests verify:

1. Strict separation of serializable intent from password-bearing input; unknown password fields rejected; password length bounds; no `Debug`, `Serialize` or `Clone` on credential-bearing types.
2. Wrong credential families, customer actor, forbidden Admin-to-Super-Admin grant, malformed input, current recovery scope and per-actor 20/hour limit.
3. Audit failure before preparation leaves no intent. Concurrent prepare replay creates one intent; changed payload conflicts; normalized same-email concurrent intent is blocked; role/access UUID reuse conflicts.
4. Attempt/audit commits before provider call. The fake acquires the same authority lock during its call, demonstrating no transaction is held across provider I/O. Audit failure before dispatch makes zero provider calls.
5. Injected local-finalization audit failure after confirmed provider creation leaves `provider_confirmed`, no local agency/wallet, and one provider call. Replaying dispatch/finalization commits once without another create. Concurrent finalization returns the same identity.
6. Password sentinel is absent from create/attempt/operation/effect/audit JSON; request hash exactly equals the hash of actor and normalized non-password intent. No password digest or full-dispatch-payload fingerprint is persisted.
7. Unknown result blocks resend, cancellation and a second same-email intent. Correlated read observation resolves it without writing. A current provider ban blocks local finalization. Wrong operation correlation cannot grant local access.
8. Expired dispatch and reconciliation leases require read recovery. Stale worker/fence completions fail; read absence cannot return the intent to dispatchable state. Provider outcomes are simulated—no real Clerk timeout/correlation semantics are claimed.
9. Proven-not-sent retries require delay and new in-memory password entry, stop after five dispatches, and safely release the email reservation on terminal cancellation. Prepared cancellation also releases capacity/reservation.
10. Actor demotion after external create prevents local role grant; current Super Admin can finish the existing verified operation. Agency suspension/version change prevents sub-user finalization without repeating provider create.
11. Existing mapped provider subject is never adopted/promoted, even with matching email. Existing retained user-wallet ownership also blocks finalization.
12. Concurrent dispatch produces exactly one fake provider account. Pending sub-user creates reserve membership capacity; cancellation frees the slot; successful finalize creates the correct membership without exceeding 100.
13. Synthetic same-email re-registration after a tombstone creates a different internal identity, agency and wallet; the old tombstone/wallet stays separate. This is not a test or implementation of live provider deletion.
14. New B2B wallet is zero with no ledger posting or API-client grant. Local completion is separate from pending provider mirror/revocation effects.
15. Intent/attempt snapshots and retention are database-enforced; production-configured dispatch remains disabled.

## Reproduction

For the creation matrix, create a **new empty** loopback database ending in `_identity_test`, set `IDENTITY_CREATES_TEST_DATABASE_URL`, then run:

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_creates -- --include-ignored --skip additive_create_upgrade_preserves_existing_evidence --nocapture
```

For the separate migration upgrade, create a **new clone** of synthetic migration-0037 evidence, whose name includes `create_upgrade` and ends `_identity_test`; set `IDENTITY_CREATE_UPGRADE_TEST_DATABASE_URL` to that clone, and run only `additive_create_upgrade_preserves_existing_evidence -- --ignored`. The final executed suite used both fresh/clone URLs and included all three tests.

Do not use production credentials, load real environment files, or reset an evidence database to satisfy test preconditions.

## Remaining gates

- Implement and verify official Clerk create/read-correlation/error contracts before connecting a writer. The fake adapter's `NotSent` guarantee and verified-correlation contract must hold for any real adapter; do not assume generic provider idempotency keys or email-only matching.
- Add verified provider correlation/webhook coordination for an account that signs in before the create result commits. The current behavior is safe denial/matching review, not silent reuse; do not erase/recreate it to recover.
- Invitations/revocation/acceptance, deletion dependency preview, durable inbox, provider-effect worker, welcome mail, denied-action auditing, recovery queue/UI and production monitoring remain pending.
- Local commit blockers after `provider_confirmed` remain visible and must be resolved through the existing operation. Never retry account creation or delete real provider accounts as a shortcut.
- Frontend identity management, business integration and live Supabase removal/cutover remain later phases. No production writer, browser account-create journey or live cutover was enabled in this slice.

# Phase 4 guarded deletion evidence — 2026-09-17

Status: staged dependency preview, durable access denial, synthetic provider deletion/recovery and retained tombstone finalization verified. **Phase 4 remains in progress.** Production deletion/provider adapters, signed durable inbox, business dependency resolution, workers and recovery/confirmation UI remain gates. This does not enable the live portal's delete action or remove its existing Supabase dependency.

Existing uncommitted work, Clerk login, Next preview/business gates and environment secrets were preserved. No real account, invitation, financial record or live database was deleted or changed. Provider deletion in these tests was an in-memory fake; all PostgreSQL writes used dedicated loopback synthetic databases.

## Implementation and scope

- Migration `0040_portal_identity_deletions.sql` adds retained deletion intents and fenced immutable attempts, transition guards and an irreversible `deleting` status guard. It performs no user/data import, purge or external call.
- `src/identity/deletions.rs` implements preview/prepare/dispatch/query/finalize plus internal read reconciliation. Only Super Admin can globally delete; an active stored B2B owner can remove their own sub-user. Admin, unrelated owners and self-delete are denied. The serialized last-active-Super-Admin protection is retained.
- Preview returns target role/status/version, owned agency identity, typed blockers and a review fingerprint. Preparation recomputes the preview, rechecks authority and atomically commits the journal/audit, user access denial and owned-agency suspension before any provider call. Exact replay is stable; changed payload/version/review conflicts.
- Global deletion retains the existing 10/hour allowance; owner removal shares the 40/hour access bucket. UUID reservations now include the deletion journal across other identity workflows. A target can have only one journal; recovery continues that operation rather than creating another deletion.
- Provider attempts commit before network I/O, with opaque claim token, incremented fence, 30-second lease and 10-second provider deadline. Only proven-not-sent calls retry, after 30 seconds and at most five attempts. Unknown outcome, expired lease or exhausted attempts retains local denial and requires read-only reconciliation. No cancellation/reopen shortcut exists.
- Confirmed provider deletion is stored separately from local completion. Finalization rechecks current recovery scope and dependencies, retains the same identity/Clerk subject/contact fields, sets a non-login tombstone, archives an eligible owned agency and commits audit. Historical memberships stay; deleted sub-users no longer consume live member capacity in create/invitation checks.
- Existing wallet/client/booking/financial references are **conservative blockers**, even if settled or zero. Non-deleted sub-users and pending provider work also block. This is necessary while legacy business writers lack canonical identity coordination. Safe dependency resolution is still pending; deleting historical rows is not a supported way to make an account eligible.
- A late legacy client link blocks dispatch/finalization. Authorized recovery/query/provider-result recording revokes discovered credentials, machine tokens and prebooking sessions while retaining the client/reference and blocker. This contains discovered access; it is not an atomic barrier against unported writers and does not justify production activation.
- HTTP routes inherit bridge token verification, current actor provider lookup, 4 KiB body cap, 15-second deadline and no-store responses. Environment runtime has no deletion provider and returns `IDENTITY_PROVIDER_WRITE_DISABLED`. There is no caller-supplied provider subject, success flag, purge option or force-complete endpoint.

## Verification executed

The dedicated cluster ran at `127.0.0.1:55451`. Fresh tests reject remote hosts, incorrect database suffixes and nonempty schemas. The upgrade test used a new clone; original evidence databases were not changed. All databases were retained and the dedicated cluster was stopped after checks.

| Check | Result |
| --- | --- |
| `cargo check --locked` | Passed without warnings |
| `cargo test --locked --quiet` | 85 ordinary tests passed; live/browser/database tests remain opt-in |
| Full deletion PostgreSQL matrix | Passed in `phase4_deletions_03_identity_test` |
| Strict deletion DTO test | Passed in ordinary suite |
| Additive migration upgrade | Passed in `phase4_delete_upgrade_identity_test`, cloned from migration-0039 `phase4_invitations_04_identity_test` |
| Upgrade preservation | 20 existing identity/agency/create/invitation/attempt/audit/financial/control/rate table fingerprints unchanged; no deletion intents or attempts fabricated |
| Foundation regression | Passed in `phase4_delete_foundation_identity_test` |
| Bootstrap/session/onboarding regression | Passed in `phase4_delete_routes_identity_test` |
| Role/access/fenced-effect regression | Passed in `phase4_delete_ops_identity_test` |
| Agency lifecycle regression | Passed in `phase4_delete_agencies_identity_test` |
| Durable create/recovery regression | Passed in `phase4_delete_create_identity_test` |
| Durable invitation/recovery regression | Passed in `phase4_delete_invitations_identity_test` |
| `cargo fmt --check`, `git diff --check` | Passed |

Earlier deletion runs `_01` and `_02` passed and remain retained. Subsequent review added revocation of late legacy client links and final coverage for retained-membership capacity, outstanding create work and credential/session invalidation; the latest `_03` run passed those additions.

The deletion matrix verifies:

1. Wrong credential families and provider-banned actors fail at the HTTP boundary. Admin, self-delete, unrelated owners and sub-users cannot delete. Preview itself creates no journal or identity version change. Request DTO rejects missing review token, caller provider subject, force/purge and success overrides.
2. Stale review/version and changed same-ID payload conflict. Preparation audit failure leaves no journal or partial status change. Concurrent exact preparation creates one intent. Create/invite UUID collisions with deletion are rejected.
3. Deleting targets immediately resolve to denied canonical session state. Ordinary access reactivation and direct database restoration fail. Exact replay remains usable through completion without charging another action.
4. Dispatch-audit failure makes no provider call. The synthetic provider verifies durable attempt and deleting status, then acquires the authority lock during its call, proving no transaction remains open across provider I/O. Concurrent dispatch performs one deletion.
5. Completion audit failure preserves provider confirmation and deleting status; concurrent/replayed finalization makes one tombstone without another provider call. Contact fields and immutable subject remain. Hard-delete or tombstone restoration is rejected. Same-email registration creates a separate internal user.
6. Unknown result cannot resend. Wrong operation correlation cannot complete. Read reconciliation uses the expected fence; stale/expired dispatch and read responses cannot overwrite current evidence. A still-existing provider user never proves deletion or permits automatic resend.
7. Proven-not-sent retries enforce delay and five-attempt cap while preserving access denial. Exhaustion is visible as unresolved. Provider success followed by local result-audit failure recovers by exact-subject read without another delete.
8. Dependency added after preparation blocks dispatch. Dependency arriving during provider I/O blocks local completion, leaving provider-confirmed/denied state for review. Discovered client access is disabled and credentials/tokens/prebooking sessions are revoked; history/reference rows remain.
9. Owner deletion is blocked by non-deleted sub-users. Scoped owner removal retains the deleted sub-user membership; eligible owner deletion suspends then archives the same agency. Owner suspension during recovery removes their authority; Super Admin can continue the same journal after outstanding effects are resolved.
10. Zero-balance user wallet, wallet request actor, draft owner/creator, booking/client ownership and pending role effects/create/invitation work all produce blockers. Wallet account, wallet request, booking and draft rows remain present after all tests.
11. Tombstoned membership releases capacity without erasing history: at 100 retained membership rows but 99 live members, one new invitation reserves the final slot; a further invitation is rejected.
12. Concurrent Super Admin deletion attempts serialize so one actor loses authority before the reciprocal operation can commit. Existing foundation last-admin concurrency tests also passed under migration 0040.
13. Shared hourly rate limit rejects another preparation while allowing exact replay. Journal/attempt deletion, retargeting and completed-evidence rewrites are database-rejected. Production-configured dispatch remains disabled.

## Reproduction

For the lifecycle matrix, create a **new empty** loopback database ending `_identity_test` and set `IDENTITY_DELETIONS_TEST_DATABASE_URL`:

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_deletions -- --include-ignored --skip additive_deletion_upgrade_preserves_existing_evidence --nocapture
```

For upgrade, create a **new clone** of synthetic migration-0039 evidence. Its name must contain `delete_upgrade` and end `_identity_test`. Set `IDENTITY_DELETION_UPGRADE_TEST_DATABASE_URL` to that clone:

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_deletions additive_deletion_upgrade_preserves_existing_evidence -- --ignored --nocapture
```

The six previous identity PostgreSQL suites ran explicitly with separate fresh URLs and `--ignored --skip additive_`. Their older migration-specific upgrade cases were not rerun; this slice's migration boundary is covered by the new clone test. Never point these tests at real databases or use real Clerk deletion credentials.

## Next work and activation gates

- Continue Phase 4 with the signed durable provider inbox and official provider adapter contracts. Verify authentication, exact-subject permanent absence, callback correlation, invitation acceptance/expiry and error classification before implementing real writers; a generic HTTP 404/timeout must not become deletion proof.
- Add bounded worker/queue discovery, operational recovery UI, mail delivery recovery and external provider-deletion containment through the inbox. No fake confirmation or caller override should bypass retained evidence.
- Define reviewed dependency-resolution paths and coordinate every client/wallet/booking writer with canonical identity denial. Current conservative blockers deliberately prevent deleting zero-wallet/settled-history accounts too. Do not lift them or erase history to simulate completion.
- Wire the existing delete UI to display affected target/agency/blockers and obtain confirmation using a fresh preview. Production deletion, live browser parity and actual activation remain unclaimed.
- Keep PII redaction separate from tombstoning and preserve account/agency/client/wallet/history references. Do not reopen uncertain deletions or reuse old identity keys when recovering or registering the same email.
- Preserve Clerk login, existing uncommitted work and all frontend preview/business gates. No real account deletion or live cutover is authorized by this evidence.

# Rust identity phases 6–8 — staged implementation and local verification

Date: 2026-09-17. Rust base HEAD `e65463c`; Next base HEAD `7caaf8c`. Work remains uncommitted in both existing worktrees. Earlier unrelated and identity/wallet changes were preserved. [Source fingerprint manifest](RUST_IDENTITY_PHASE6_8_SOURCE_MANIFEST_2026-09-17.json) records this delivery's source files rather than implying a new commit.

**Result:** Phase 6 business integration, Phase 7 ported UI/local setup, and Phase 8 isolated acceptance/failure review are complete for the staged implementation. Phase 9 backup/restore/readiness rehearsal passed, but **actual target configuration, production activation code/gates and live cutover are not complete**. No live deployment, real account/data deletion, financial import, real invitation/mail/SMS/asset write or real `.env` modification occurred.

## Implemented behavior

- Dedicated identity business bridge with a closed internal route allowlist and request-local canonical authority. Wallet owner/display/receiver, booking owner/assignee, passenger actor, client and directory resolution use Rust. Actor/target/client authority is rechecked in local mutation transactions; no identity lock is held during provider/supplier calls.
- Migration 0045: separate user/version-bound search sessions and revocation trigger. Canonical `sti_` credentials cannot Book/Issue; accepted pricing rechecks current authority in the transaction.
- Wallet deposit/review/shared-agency statement and settings use canonical identity, while new agency wallets remain zero balance. Explicit API client provisioning links only the same canonical agency wallet. Account creation does not enable machine API access.
- Native hold prepare/accept/submit/read and ticket ownership run through the canonical wrapper. Admin/Support/Accounts read existing native bookings; agency sub-user reads use the current canonical owner. Supplier cost remains Super Admin only. Existing supplier state-machine settlement remains available after dispatch.
- Account roster, filter/date/sort/count/page, create/invite/access/role/agency-reactivation, removal preview/confirmation, scoped owner invitation, persistent operation ID/status and recovery are wired at existing dashboard URLs. Password is transient dispatch input. Company/profile/staff/application/documents use the Phase 5 workspace; Clerk's own security widget remains.
- Recent-sign-in sorting and avatars hydrate only authentication presentation fields for Rust-authorized IDs; forged Clerk metadata never changes role, name, status or membership. Search/results/checkout/airport routes and original dashboard shell are wired for the ported native Rust flows.
- Dedicated wallet notification worker capability, claim-bound canonical recipient resolution, durable immutable plans and acknowledgement/retry rules. Preview transports permit only local test SMTP and reject live SMS. Existing Clerk authentication-email relay preserves signed content/message ID and avoids duplicate legacy application welcome.
- Legacy identity DB entry points, old account actions, generic admin transport and middleware gates reject canonical-preview fallback. Configuration/read failures stay visible rather than writable empty forms. Setup check reports schema/bootstrap/recovery status without printing credentials or activating anything.

## Executed verification

| Check | Result / retained fixture |
| --- | --- |
| Ordinary Rust suite | **88 passed**, no failures; ignored database/live tests are not included in that count |
| Rust format and strict Clippy | `cargo fmt --all -- --check`; `cargo clippy --lib --tests -- -D warnings` passed |
| Prior identity/business/wallet full matrices | **14 explicitly executed matrices**, all passed, each a new DB: `identity_exit_01_<test>_<suffix>`. Tests: identity_business, identity_phase5, identity_runtime, identity_deletions, identity_invitations, identity_creates, identity_agencies, identity_profiles, identity_operations, identity_api, identity, wallet, wallet_notifications, database |
| Final expanded canonical business matrix | `identity_business_exit_04_identity_test`: zero wallet/shared sub-wallet/deposit/finance review/statement; forged actor/owner/receiver; cross-agency passenger/client access; receiver/directory privacy; scoped roster; worker token separation, lease-bound contacts, immutable notification plan/ack replay; OpenAPI access; suspension during external receiver lookup blocks deposit before commit; retained balance and revoked search token |
| Final canonical booking matrix | `identity_bookings_exit_03_identity_test`: runs existing full mocked commercial fixtures, then actual canonical prepare/accept/concurrent Book. One supplier dispatch, immutable stored creator/owner, canonical branding, owner/sub/staff receipt scope, cost privacy, timeout `outcome_unknown`, no resend, suspension with preserved booking history |
| Additive 0044→0045 | `identity_business_upgrade_01_identity_test`, cloned from synthetic `phase5_complete_03_identity_test`: **75 existing table fingerprints unchanged**, no inferred sessions/imports |
| Backup/restore | `identity_restore_phase6_01_identity_test`: **77 public table fingerprints and all sequences match** the backed-up upgraded source; source unchanged |
| Actual restored-schema startup | Built Rust binary started from an empty temporary directory with only synthetic config. HTTP health/readiness plus `check-identity-local.mjs` passed; bootstrap true, current schema true, zero pending/unknown operations, writers/mail/inbox disabled, activation unavailable. Process then stopped |
| Next adapter/regression scripts | **14 offline scripts passed**: identity, authority, runtime, profiles, phase5, business, roster, auth-email, wallet, holds, prebooking, notification-providers, b2c-disabled, application-profile |
| Chrome journeys | **3 suites passed**: account management, Phase 5 profile/application/document/name journeys, and recovery UI. Actual components, local fixture APIs, blocked external requests, no browser errors; account screenshot uses actual Tailwind theme |
| Next static checks | TypeScript and targeted ESLint passed |
| Next production build | Passed in a separate temporary source copy, excluding real `.env*`, `.next`, outputs and inherited integration secrets; webpack compile, TypeScript and page build passed. This is a build check, not production authority activation |

Database suffixes for the 14-matrix batch: `_identity_test` except wallet (`_wallet_test`) and wallet_notifications (`_notification_test`). The legacy `database` matrix deliberately corrupts its own migration checksum at the end to prove schema readiness rejection; do not use that fixture as a ready deployment or backup source.

Earlier unsuccessful intermediate fixtures are retained, never overwritten/dropped. They exposed a time-dependent assertion, a missing email HTML fixture, the intentional extra-role denial now returning 403, and the existing durable hold timeout returning HTTP 200 with `outcome_unknown`. Final assertions test the actual safety contract rather than treating that durable unknown snapshot as failure or proof that nothing was sent.

## Evidence artifacts and reproduction

- [Account Chrome screenshot](../../../shopontravels/docs/evidence/identity-accounts-fixture.png).
- [Runbook](../RUST_IDENTITY_RUNBOOK.md): setup, configuration, matrix commands, recovery and activation boundary.
- [Dependency inventory](../RUST_IDENTITY_DEPENDENCY_INVENTORY.md): migrated, guarded and explicitly unrelated legacy surfaces.
- [Verification summary](RUST_IDENTITY_PHASE6_8_VERIFICATION_2026-09-17.json): exact checks, counts and fixture names.
- Synthetic backup and restore evidence: `.local/identity-rehearsal/identity_restore_phase6_01_identity_test.{dump,json}`. They are private local artifacts, not production data. No restore occurs over an existing DB.

These tests prove local state-machine, adapter and component contracts using synthetic identities/providers. They do not prove a real Clerk sign-in/invitation, actual SMTP/Cloudinary delivery, supplier entitlement, a deployed production smoke test, or a target-database backup. Provider/supplier traffic was mocked for the matrices. Local PostgreSQL review cluster is stopped after verification; retained fixture databases remain available.

## Remaining Phase 9 gates

1. Explicitly select and inventory the actual target/operator and retained immutable business identity mappings. No email/name adoption or automatic financial import.
2. Implement/review the coordinated production activation mode/marker and writer shutdown. Current supported modes deliberately remain disabled/staged and legacy/local-preview; setting NODE_ENV to development on production is not a workaround.
3. Back up and restore the **actual selected target** separately, configure authorized production provider/mail/storage transport, reconcile pending/unknown work, bootstrap the selected operator and verify canonical reads.
4. Execute the approved target activation/smoke/monitoring plan and record evidence. The user's prior instruction prohibits live cutover and real data deletion, so this delivery did not perform them.

Unrelated legacy booking/import/refund/reissue/media/marketing features are not claimed migrated; see the inventory. Unknown provider name or mail outcomes retain the prior explicit support-review/no-blind-resend rules. Do not mark the overall identity project complete until Phase 9's actual activation gate passes.

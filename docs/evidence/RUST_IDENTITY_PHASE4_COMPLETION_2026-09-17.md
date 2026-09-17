# Phase 4 completion work — 2026-09-17

Status: Phase 4 complete as staged/isolated implementation. Existing uncommitted work was preserved. Verification used synthetic accounts, loopback HTTP fixtures and disposable PostgreSQL only. No real account deletion, mail, provider mutation or live cutover occurred. Phases 5–9 remain pending.

Completion checklist:
- [x] Signed Next webhook boundary and durable idempotent inbox; stale/duplicate events cannot grant or resurrect; provider deletion denies linked access.
- [x] Official Clerk transport adapters for create/invite/expiry/accept/revoke/delete/effects, tested against loopback HTTP fixtures; password-safe, bounded and fail-closed.
- [x] Bounded worker scheduling, recovery scans, cursor queue/detail API and recovery UI; unknown outcomes observed, never blindly resent.
- [x] Durable branded invitation/account/welcome mail with single-attempt transport and ambiguous-outcome handling.
- [x] Deletion dependency resolution for retained settled history with atomic barriers against new business writes; no historical purge.
- [x] Denied-action audit and runtime/config documentation; all provider writes remain off by default and tests perform no real writes.
- [x] Failure/concurrency/upgrade tests, frontend verification and Phase 4 exit evidence.

Official contracts consulted (implementation uses operation correlation, never metadata role authority):
- https://clerk.com/docs/reference/backend/user/create-user — unique external ID and backend-only private metadata for account correlation.
- https://clerk.com/docs/reference/backend/user/get-user-list — bounded external-ID/user/email filters; email alone does not identify an operation.
- https://clerk.com/docs/reference/backend/invitations/create-invitation — notification suppression, canonical redirect, copied backend-controlled public metadata.
- https://clerk.com/docs/reference/backend/types/backend-invitation — pending/accepted/revoked/expired states.
- https://clerk.com/docs/reference/backend/invitations/get-invitation-list — bounded query/status pagination; absence is not proof a prior send failed.
- https://clerk.com/docs/reference/backend/user/delete-user — exact-subject DELETE.
- https://clerk.com/docs/reference/backend/sessions/get-session-list and /revoke-session — enumerate/revoke sessions; subsequent read confirms absence of active sessions.
- https://clerk.com/docs/reference/backend/user/update-user-metadata — merge only owned keys, preserving unrelated metadata.
- https://clerk.com/docs/reference/backend/verify-webhook — SDK signature verification of raw request.


## Delivered implementation

- Rust `src/identity/clerk.rs`: official BAPI transports with fixed official origin, no redirects/automatic retries, capped response bodies, exact subject/operation correlation and conservative unknown outcomes. Passwords are only sent in memory; stored create intent/audit remains password-free. Clerk role metadata is never canonical authority. Invitation expiry, accepted-user correlation and metadata/session effects are supported.
- `src/identity/inbox.rs` and Next `app/api/identity/events/route.ts`: raw-body SDK signature verification, separate service capability, idempotent durable envelopes, collision rejection, bounded retries/dead letters and irreversible provider-deletion evidence. Signed deletion immediately denies local access, even before worker processing; stale events cannot resurrect identities. Provider reads refresh contact data only.
- `src/identity/mail.rs`, shared provisioning/onboarding hooks and Next `app/api/identity/mail/route.ts`: atomic branded invitation/account/welcome intent, independently tracked recipient/archive jobs, 90-second fenced leases, a single-use transport start and one SMTP attempt. Timeout/crash/uncertain acceptance remains unknown without automatic resend. No password enters an email. Local eligibility is rechecked before transport.
- `src/identity/recovery.rs` and main runtime: bounded, rotating lifecycle scans, provider-effect/mail dispatch and inbox processing, shutdown handling, root-scoped cursor queue and provider-read/finalization commands. No browser success override or blind resend. Password creates require password re-entry after a proven-not-sent result; the worker cannot recover a password.
- Next `/identity-recovery`, `/api/identity/recovery`, `/identity-invite/[id]` and verified invitation acceptance bridge: current session and Rust authority, same-origin mutation checks, no caller actor injection, pending-result/error/fence visibility and cursor pagination. The status page links active Super Admins to recovery. Existing business/account-management preview gates remain.
- Migrations `0041_portal_identity_runtime.sql` and `0042_identity_business_barrier.sql`: durable evidence and deletion coordination. New finance/booking/client/credential/session dependencies serialize with deletion under the same transaction lock. Zero wallets and settled history are retained; unresolved funds, holds, requests, bookings/drafts and live members still block. Owner wallets freeze and linked access is revoked. Terminal subjects cannot acquire new dependencies or reactivate credentials.
- Denied/conflicting authenticated bridge actions append immutable typed audit. Runtime defaults stay disabled. Official writers require explicit `test`, `sk_test_`, and a loopback `_identity_test` database. Mail additionally requires a loopback Next origin and disposable loopback SMTP fixture. Live provider keys cannot enable writers. Real environment files were not edited.

## Executed verification

| Check | Result / retained evidence |
| --- | --- |
| `cargo test --lib --tests --no-fail-fast` | 86 ordinary tests passed; opt-in database/UAT tests were not silently counted as executed |
| `tests/identity_runtime.rs::official_adapter_loopback_contract` | Exact account correlation, password body isolation, safe initial metadata, duplicate/mismatched evidence, invitation notify=false/redirect/expiry/accepted-user correlation, wrong-subject deletion, metadata PATCH, session revoke/readback, partial failure without retry, HTTP errors and oversized response rejection; loopback only |
| Runtime PostgreSQL matrix | `phase4_runtime_06_identity_test`: signed envelope collision/replay/out-of-order/unmapped deletion, contact-only updates, five-attempt dead letter and scoped retry, signed-delete + 404 proof, mail claim races/once-only start/retry delay/stale fence/crash/unknown-no-resend, expiry blocking, cursor uniqueness, denied audit, coordinator effect/mail dispatch, business-write race and full zero-wallet/settled-ledger/client-retaining deletion |
| Seven prior identity matrices with migration 0042 | `phase4_runtime_{delete,invite,create,ops,agency,api,foundation}_04_identity_test`: all passed, including prior lifecycle concurrency, crash, last-root/self/scope/agency/capacity, password isolation and retained identity tests |
| Clone upgrade 0040 → 0042 | `phase4_runtime_upgrade_02_identity_test`, cloned from synthetic `phase4_deletions_03_identity_test`: **63 existing tables hashed identically** before/after; new mail/inbox tables empty; no historical backfill/send |
| Wallet regression matrix | `phase4_barrier_wallet_test`: wallet integrity, financial posting, concurrency, rollback, frozen capture, request/review, reports and runtime DB role tests passed with the identity barriers |
| Wallet notification regression matrix | `phase4_barrier_notification_test`: existing notification claims/retries/unknown/stale tokens passed; no financial changes |
| Next typecheck and selected ESLint | `tsc --noEmit --pretty false` and ESLint on all new/changed identity runtime/UI routes: passed |
| Existing offline Next bridge/authority scripts | `verify-rust-identity.mjs`, `verify-rust-identity-authority.mjs`: passed, including legacy gates and status-page rendering |
| New offline Next runtime script | `verify-rust-identity-runtime.mjs`: **actual installed Clerk SDK** verifies valid signatures; rejects invalid/stale signatures and oversized bodies; sanitizes envelopes; rejects actor/origin injection; exercises branded mail, archive copy and single-use start; no real SMTP/network |
| Actual React queue in headless Chrome | `verify-rust-identity-recovery-ui.mjs`: stale-fence alert, provider observation, saved-result refresh, pagination/empty state, unknown-mail no-resend and zero browser errors; all requests restricted to loopback |
| Formatting / whitespace | `cargo fmt --all --check`, `git diff --check`: passed |

Database checks total **11 explicitly executed matrices/upgrade checks**, in addition to the 86 ordinary Rust tests. Intermediate fixture/test failures were corrected and rerun on fresh databases. In particular, the real SDK test caught that its returned event object omits the raw event timestamp; timestamp extraction now occurs from the already verified raw body. Earlier audit-count assertions now count successful onboarding events separately from newly recorded denials. Earlier deletion tests now assert atomic late-write rejection and retained-client revocation instead of the superseded blanket reference blocker.

Browser fixture screenshot: [recovery queue](../../../shopontravels/docs/evidence/identity-recovery-fixture.png). This screenshot uses synthetic loopback data and the actual queue component with fixture styling; it is not a claim that a real Clerk session or live portal journey was exercised.

Reproduction: use a **new empty** loopback database ending `_identity_test` for each ordinary identity matrix and its documented environment variable; run `cargo test --test <matrix> -- --ignored --skip additive_`. Runtime uses `IDENTITY_RUNTIME_TEST_DATABASE_URL`; upgrade uses a **clone** of migration-0040 synthetic evidence and `IDENTITY_RUNTIME_UPGRADE_TEST_DATABASE_URL`, running `cargo test --test identity_runtime additive_runtime -- --ignored --nocapture`. Never point these at real data. The preserved databases include earlier intermediate evidence; do not reuse a populated test database as an empty fixture. The dedicated local review cluster is stopped after verification.

## Phase boundary and operational limits

Phase 4 is complete locally, with production activation deliberately disabled. The official adapters were tested against controlled HTTP contracts, not real Clerk mutation endpoints; SMTP was mocked, not sent to real recipients. Existing account create/delete pages still use the legacy store until later frontend/business integration and controlled cutover. Full profiles/documents, application review, company/staff parity and UI journeys are Phases 5–8; real tenant/configuration verification, deployment rehearsal and explicit activation are Phase 9.

Unknown mail/provider outcomes may remain queued for support review; completion does not mean fabricating successful delivery. No generic force-success, email-based identity adoption, tombstone reuse or automatic Super Admin promotion exists. An externally deleted last Super Admin requires a separately reviewed recovery/restore, not clearing bootstrap evidence. Read the [runtime configuration and recovery runbook](../PORTAL_IDENTITY_API.md#phase-4-runtime-mail-and-recovery) before configuring a disposable provider/SMTP fixture. The new business barrier is deletion containment; it does not replace the full business actor/recipient authorization work in Phase 6.

**Next: Phase 5 — agency/sub-user profile and document parity, B2B application review and canonical branding.** Keep existing uncommitted work, staged defaults and all live-data restrictions.

# Phase 9 preparation — read-only preflight and coordinated maintenance

Date: 2026-09-17. Rust base HEAD `e65463c`, Next base HEAD `7caaf8c`. Both existing worktrees remain uncommitted; prior changes were preserved.

**Result:** operator preflight, complete queue/mapping review, private review/drift artifacts and maintenance controls are implemented and verified locally. **Phase 9 is not complete.** Canonical production mode/marker, actual target/operator configuration, target backup/restore and authorized activation/smoke remain outstanding. No real `.env`, account/data, provider, supplier or live deployment was changed.

## Implemented

- `src/identity/preflight.rs`: dedicated operator-authenticated POST endpoint. Selected local active Super Admin, schema/bootstrap, 14 unresolved work queues, eight mapping issue categories and SHA-256 fingerprints of 11 mapping tables. No provider lookup, PII/credential output, mutation, automatic adoption or activation.
- Readiness now includes mail, assets, inbox/dead-letter, reconciliation states, wallet notifications/reservations/requests and booking/issue/cancellation work. Ticket state uses `flight_ticket_outcomes`, retaining verified issue/nonissuance recovery; manually resolved bookings do not count as unresolved. Queue counts can overlap across one logical operation.
- Both inspection endpoints use repeatable read/read-only transactions and bypass mutation-based rate-limit/audit middleware. Missing/mismatched schema, database outage and the 100,000-row mapping review bound fail closed.
- `scripts/identity-preflight.mjs`: explicit selected origin/operator; no dotenv; HTTPS or loopback-only HTTP; strict bounded response and freshness checks; new 0600 output files without overwrite; target/operator binding and mapping drift comparison. No raw Clerk subject, provider secret or personal fields in the saved artifact. File digest is corruption detection, not a signature.
- `PORTAL_IDENTITY_MAINTENANCE=true`: Rust business/legacy/machine traffic returns 503/no-store/Retry-After; only liveness and authenticated read-only review remain. Identity workers pause and cleanup is not started. `/health/ready` intentionally fails while frozen.
- `SHAPON_IDENTITY_AUTHORITY=maintenance`: Next matched application/API/actions fail before legacy/canonical work, with 503/no-store/Retry-After. Existing legacy and local preview behavior remain tested. This is process configuration requiring coordinated drain/restart, **not** a durable authority marker or proof that old/external workers are stopped.

## Verification

| Check | Result |
| --- | --- |
| Ordinary Rust tests | 88 passed, no failures; ignored database/live tests excluded |
| New PostgreSQL matrix | Passed on new `identity_preflight_03_identity_test`; SELECT-only role with `default_transaction_read_only=on` successfully exercised actual router endpoints; all table hashes unchanged by reads/paused work |
| Authorization and failure matrix | Bridge/missing/wrong tokens denied on operator preflight; extra fields/invalid subject rejected; wrong selected operator remains blocked; closed database returns 503; maintenance blocks bootstrap/session/operations/events/mail/business and legacy/machine routes |
| Queue/mapping coverage | Seeded unknown mail, unknown asset, dead-letter event, unmapped retained client and unmapped wallet correctly reported; no auto-repair/relink or clear result |
| Worker pause | Paused tick succeeds even after its pool is closed: no database/provider work attempted |
| Business regression | Full existing `identity_business` matrix passed on new `identity_preflight_business_01_identity_test` |
| Actual binary and HTTP CLI | Final Rust binary launched from empty temporary directories with an explicit synthetic-only environment; readiness/preflight, blocked routes, repeated review and no-drift comparison passed |
| Source preservation | Both actual HTTP rehearsals verified **all 77 public tables and sequences unchanged**, including after a real worker tick; processes terminated normally |
| Retained booking fixture review | Existing synthetic `identity_bookings_exit_03_identity_test` was inspected without mutation. 5 manually resolved bookings and 5 verified issue recoveries were excluded correctly; 15 unresolved bookings and 11 unresolved effective ticket issues remain reported |
| CLI contract | Loopback-only fixture tests passed for private/no-overwrite output, strict/malformed response rejection, stale timestamp, backlog consistency, redaction, mapping drift, missing maintenance, invalid credential/config and upstream failure |
| Next authority/maintenance | `verify-rust-identity-authority.mjs` passed, including matched dashboard, webhook, scheduler, account API and Server Action maintenance denial; legacy/preview regressions and status rendering passed |
| Static/build checks | Rust format, strict Clippy, Rust binary build, Next TypeScript and targeted ESLint passed; both worktrees passed `git diff --check` |
| Final post-fix Rust rerun | Full `cargo test --lib --tests --no-fail-fast` passed with loopback permissions enabled; 88 non-ignored tests passed and all database/live tests remained explicitly ignored |

Ordinary Rust log: `/tmp/identity-preflight-rust-tests.log`; final DB matrix: `/tmp/identity-preflight-matrix.log`; business regression: `/tmp/identity-preflight-business.log`; Clippy/build: `/tmp/identity-preflight-{clippy,build}.log`; Next static checks: `/tmp/identity-preflight-next-static.log`. The earlier Phase 6–8 production Next build remains historical evidence; this change was checked with TypeScript, targeted lint and the authority/maintenance suite rather than claiming another production frontend build.

Final retained private artifacts:

- `.local/identity-rehearsal/phase9-preflight-03/{review-1.json,review-2.json,evidence.json,server.log}`
- `.local/identity-rehearsal/phase9-booking-review-01/{review-1.json,review-2.json,evidence.json,server.log}`

Earlier disposable attempts/roles/databases were retained. No existing database was overwritten or dropped. The local PostgreSQL review cluster was stopped after verification.

Reproduce using the [runbook](../RUST_IDENTITY_RUNBOOK.md), `scripts/test-identity-preflight.mjs`, the ignored `identity_preflight` matrix on a **new** DB, and `scripts/test-identity-preflight-runtime.py` on a retained synthetic fixture. [Source hashes and verification summary](RUST_IDENTITY_PHASE9_PREFLIGHT_VERIFICATION_2026-09-17.json) bind this uncommitted slice to its files.

## Pinned rollout implementation

The durable migration-0046 rollout marker, canonical release/target pin and fenced `activate`/`pause`/`resume` endpoint are now implemented. `src/identity/rollout.rs` fails closed on missing/mismatched markers, release headers, stale worker revisions and in-flight transaction changes. The operator-only `scripts/identity-rollout.mjs` plan/apply workflow binds the preflight mapping digest and external backup/restore/writer-shutdown/configuration evidence, refuses overwrite/retry and records a confirmed or unconfirmed result. It never performs an automatic rollback. The synthetic `identity_rollout_01_identity_test` matrix passed all marker, concurrency and fencing assertions.

[Rollout source manifest](RUST_IDENTITY_PHASE9_ROLLOUT_SOURCE_MANIFEST_2026-09-17.json) records the marker/preflight source fingerprints for this uncommitted slice.

## Remaining gates

The preflight always returns `activation_ready:false`; the rollout plan requires independent evidence before it can be applied. CLI exit 0 means a clear, frozen, unchanged review snapshot only. External operator verification, actual target backup/restore, replica/scheduler shutdown and deployment configuration are not established by these reads. The selected real target/operator was not supplied for this run. The canonical mode/marker implementation is available for review but remains unconfigured and unperformed on a real target. The prior no-live-cutover instruction remains in force; no activation authorization was inferred from “continue.”

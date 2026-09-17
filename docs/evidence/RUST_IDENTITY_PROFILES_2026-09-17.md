# Phase 5 profile/staff text and scoped branding slice — 2026-09-17

Historical slice; superseded by [Phase 5 completion](RUST_IDENTITY_PHASE5_COMPLETION_2026-09-17.md).

Status at the time: this slice is implemented and verified in isolated staging. **Phase 5 remains in progress.** Application submission/review/atomic approval, document references/uploads/signed reads, canonical logo, sub-user rename parity and frontend journeys are still pending. Phases 6–9 remain pending. Clerk login and all existing live/preview authority gates remain in place.

## Source verification and behavior

Reviewed the saved Phase 4 completion evidence and current Rust lifecycle/authority code before editing. Reviewed Next `lib/profile.ts`, `lib/staff.ts`, `lib/company-profile-access.ts`, `lib/db/profiles.ts`, `lib/db/staff.ts`, `lib/application-profile.ts`, `lib/dashboard/sub-user-guard.ts`, `lib/roles.ts`, `lib/rate-limit.ts`, profile/staff/user actions and identity bridge.

Current executable source makes B2B owner/sub-user profile writes read-only, including personal/company/bank text. This supersedes older comments implying self-edit permission for some business fields. The Rust slice deliberately preserves the stricter current behavior and manager correction permissions. Staff records separately retain the exact self-or-actual-owner rule; global managers are not implicitly added.

## Implementation

- `migrations/0043_portal_identity_profiles.sql`: additive typed/size-bounded JSON text records, monotonic versions, staff removal tombstones, immutable keys, no DELETE/TRUNCATE resets and immutable payload-hash mutation journal. No existing table backfill or provider/business mutation.
- `src/identity/profiles.rs`: 23 profile text fields and six staff fields, role-dependent field allowlists, current identity and agency scope, bootstrap/provider tombstone checks, optimistic profile **and** identity versions, explicit clearing, partial merge, normalized idempotent replay, safe transactional read/write audit and actor rate buckets.
- `src/identity/api.rs`: three bridge-only routes with provider lookup, existing bounded requests/rate limits/denial audit, no-store responses and distinct OpenAPI schema names.
- Branding projection derives from the canonical owner and exposes six company contact fields only. Sub-user company text cannot override owner branding; private bank/passport/license data is excluded.
- Next `lib/identity/profiles.ts` validates payload/response, binds target/kind/versions, resolves a fresh Clerk subject, checks current-role field sets and rejects forged subjects/roles/owners. `runtime.ts` adds only the three server transport actions. There is no legacy fallback or empty-on-error path, and no new browser mutation route or live form wiring.

Read permissions require an active actor; managers may correct nonterminal suspended targets. Onboarding applicants need the future application-specific workflow. Profile email/name are application data and do not alter Clerk login or registry authority. File fields are intentionally excluded pending asset-ownership/signed-read work. Partial patches share the existing 4096-byte bridge body bound.

## Executed verification

| Check | Outcome |
| --- | --- |
| `cargo test --lib --tests --no-fail-fast` | 87 ordinary tests passed; ignored opt-in tests were not counted as executed |
| `identity_profiles` explicit matrix | Passed on final fresh `phase5_profiles_04_identity_test`: every role, two agencies, own/manager/owner boundaries, all 23+6 fields, normalization, explicit clear, replay mismatch/current snapshot, stale identity/profile versions, concurrent writes, staff tombstone/recreate, suspended agency, no authority/wallet writes, dedicated credential/forged envelope/provider denial/body bound, audit rollback, unavailable-store denial, rate ceiling, DB field/delete/truncate/journal guards and no profile PII in audit |
| 0042 → 0043 upgrade | Passed on final `phase5_profiles_upgrade_02_identity_test`, a clone of synthetic `phase4_runtime_06_identity_test`: **67 existing table hashes unchanged**, both new tables empty |
| Existing identity operation/runtime matrices | Passed on final fresh `phase5_operations_02_identity_test` and `phase5_runtime_02_identity_test`; role/access/effect recovery, inbox/mail and retained-business-write barriers preserved |
| `node scripts/verify-rust-identity-profiles.mjs` | Actual adapter passed offline: subject binding, strict fields/versions, target binding, explicit clear, missing vs outage, conflict, branding privacy, revoked sessions, disabled preview |
| Existing Next scripts | `verify-rust-identity.mjs`, `verify-rust-identity-authority.mjs`, `verify-rust-identity-runtime.mjs`, `verify-company-profile-access.mjs`, `verify-application-profile.mjs` passed |
| TypeScript and ESLint | `npm run typecheck`; targeted lint of new adapter/test and changed runtime passed |
| Rust formatting / diff whitespace | Targeted rustfmt and `git diff --check` passed |
| Clippy | Normal targeted Clippy completed with six pre-existing warnings: five nested-if suggestions in `api.rs`/`clerk.rs`, one tuple type-complexity warning in `mail.rs`. Strict `-D warnings` fails on those six. No warning in the new profile module/test; unrelated prior work was preserved. |

All database tests used dedicated loopback PostgreSQL at `127.0.0.1:55451`, synthetic identities and fake providers. No actual Clerk write, email, Cloudinary operation, live database change, real-account deletion or cutover ran. Real environment files were untouched. Earlier intermediate fixture databases are retained; never reuse a populated fixture as an empty test database. The review cluster is stopped after verification.

Reproduce the profile matrix on a **new empty** loopback `_identity_test` database with `IDENTITY_PROFILES_TEST_DATABASE_URL` and `cargo test --test identity_profiles profile_staff_scope -- --ignored --nocapture`. For upgrade, use a **clone** of synthetic migration-0042 evidence with `IDENTITY_PROFILES_UPGRADE_TEST_DATABASE_URL` and `cargo test --test identity_profiles additive_profiles -- --ignored --nocapture`.

## Continue next

Implement versioned B2B applications (submit/resubmit/read/review/accept/reject), atomically granting role/agency/membership/profile transfer on approval with durable provider/mail work. Then add validated document references and canonical logo, preserve existing Cloudinary byte/type/size/private-link behavior, finish sub-user rename parity and prove permission/concurrency/failure cases. Future application transfer must respect existing profile keys, including explicit empty strings; never restore deliberately cleared fields during reads. Keep business and frontend preview gates until their later integration/parity phases pass.

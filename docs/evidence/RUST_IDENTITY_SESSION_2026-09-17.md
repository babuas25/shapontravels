# Canonical session authority — Phase 3 local evidence

Phase 3 of [the requirements plan](../RUST_IDENTITY_REQUIREMENTS_PLAN.md) is implemented and verified in **isolated Rust preview**. Live activation remains Phase 9; account lifecycle and business integration remain Phases 4–6. This phase does not make the existing live account create/delete workflow independent of Supabase.

## Implementation

### Authority and presentation

- Next `lib/identity/authority.ts` resolves the provider-verified current subject through Rust on every call. There is no canonical role cache across calls and no metadata or Supabase fallback.
- The resolver distinguishes authenticated, onboarding-required, suspended, deleted, missing agency setup, unauthenticated and unavailable outcomes. A missing registry row never becomes an active customer implicitly.
- `getAccountSession()` selects Rust only under the explicit isolated preview gate. Canonical role, UUID, current versions, agency membership, stored ownership and contact snapshots come from Rust. Request-level caching is retained only for the existing legacy branch.
- `getDashboardSession()` no longer reads the development role cookie in any mode. `getDashboardPresentationRole()` is used only by DashboardShell navigation. Thus an existing development account preview cannot grant mutations or protected data access, even in legacy mode.
- Clerk remains responsible for authentication. The Phase 2 current-session and provider ban/lock checks remain in effect.

### No mixed authority during partial implementation

`SHAPON_IDENTITY_AUTHORITY` defaults to `legacy`. The new `rust-preview` value requires a non-production Next runtime, `SHAPON_IDENTITY_MODE=staged`, and a loopback HTTP Rust base URL. Unknown modes, production preview, remote base URLs and missing staged configuration fail closed. No environment file or running deployment setting was changed.

The preview is deliberately read-only at the portal boundary until its dependent features are implemented:

| Surface | Phase 3 behavior in preview |
| --- | --- |
| `GET /api/identity/session` | Resolve only the authenticated current subject; safe JSON state, private response, typed 401/403/409/503 outcomes |
| `/identity-status` | Show account/setup/suspension/deletion/outage feedback with reload/sign-out; no legacy profile form or account writes |
| Dashboard, B2B application and flight pages | Proxy routes the visitor to the status page |
| Other APIs, tRPC, old invitations, callbacks and scheduled HTTP jobs | Return 503 `IDENTITY_INTEGRATION_PENDING`; no dispatch through the preview portal |
| All non-GET/HEAD requests and requests carrying `Next-Action` | Refused by preview proxy, including action replay through another page |
| Shared business session, API Management actor, wallet actor, passenger actor | Independent integration guards block direct invocation of unported paths |
| Generic Rust business transport | Guard before admin login/network; the dedicated identity transport remains separate |
| Legacy B2B application page/action | Explicit guard before storage/upload/provider work |

These are integration gates, not finished account/business migrations. Default legacy mode retains existing business/email/webhook behavior. Rust business routes are not claimed to revalidate canonical actors yet; that remains Phase 6 and is why the preview cannot dispatch those operations.

### Rust session contract

`src/identity/api.rs` now returns stored email/first/last name with the identity snapshot so the canonical Next resolver does not hydrate from legacy profile/role readers. The request remains subject-only; no new mutation endpoint or migration was added. Session state still resolves agency and owner suspension from one database query.

The Next wire schema also rejects contradictory state/status, missing B2B agency, impossible owner flags, privileged onboarding states and mismatched subjects. Malformed backend responses produce unavailable state, never a default grant.

## Verification

All required targeted checks passed:

- Full `cargo test --locked`: **80 passed**, ordinary opt-in UAT/database suites ignored as designed.
- Explicit real-router identity integration on new `phase3_session_01_identity_test`: **1 passed**, including every role, immediate role/status/version visibility, owner/sub membership, owner suspension and agency suspension.
- `cargo fmt --check`, all-target Clippy with warnings denied and Rust whitespace check.
- Offline `verify-rust-identity.mjs` and new `verify-rust-identity-authority.mjs`.
- Existing API Management, account-security, B2C and Rust prebooking regressions.
- TypeScript `tsc --noEmit`, targeted ESLint for all changed TS/TSX modules, and frontend whitespace check.
- Actual status-page server-component rendering for verified, onboarding, suspended, deleted and unavailable states; this is **not** an authenticated browser journey.

The B2C regression contained a stale source assertion expecting the old inline flight route gate. Updated it to the existing `handlePrebooking` delegation; actual role/acceptance behavior is also covered by the API Management and Rust prebooking regressions. Its role matrix now requires the real authority role in both production and development.

The optional `verify-rust-passengers.mjs` command refused to start because no new `api_portal_local_*` database was supplied. Its safety precondition prevented all work; it is not counted as executed or passed. Full passenger/wallet business database journeys remain Phase 6/8 work. The new offline test separately proves direct passenger/wallet actor calls are gated before legacy/provider access in identity preview.

### Main commands

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked
IDENTITY_API_TEST_DATABASE_URL=postgres://ashifbabu@127.0.0.1:55451/phase3_session_01_identity_test \
  /Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_api -- --ignored --nocapture
/Users/ashifbabu/.cargo/bin/cargo fmt --check
/Users/ashifbabu/.cargo/bin/cargo clippy --locked --all-targets -- -D warnings
git diff --check
```

From `/Users/ashifbabu/Projects/shopontravels`:

```sh
node scripts/verify-rust-identity.mjs
node scripts/verify-rust-identity-authority.mjs
node scripts/verify-api-management.mjs
node scripts/verify-account-security-lock.mjs
node scripts/verify-b2c-disabled.mjs
node scripts/verify-rust-prebooking.mjs
./node_modules/.bin/tsc --noEmit
./node_modules/.bin/eslint lib/dashboard/session.ts lib/identity/*.ts lib/rust-api/transport.ts lib/api-management/server.ts lib/rust-passengers/server.ts lib/wallet/rust.server.ts proxy.ts app/api/identity/session/route.ts app/identity-status/page.tsx 'app/(dashboard)/layout.tsx' 'app/(dashboard)/actions.ts' 'app/(dashboard)/dashboard/upgrade/actions.ts' app/b2b-application/page.tsx app/page.tsx
git diff --check
```

## Isolation and remaining work

The retained disposable `.local/identity-review-db` cluster was started on `127.0.0.1:55451`; only the new empty `_identity_test` database was migrated/written. The integration test refuses a non-loopback or populated database. It uses a synthetic provider and synthetic account/agency rows. The cluster was stopped after testing; the evidence database is retained and must not be reused/reset for another run.

Existing uncommitted changes in both repositories were preserved. No real Clerk/provider account write, invitation/email, supplier/ticket dispatch, live DB migration, secret-file modification, live cutover or financial data deletion was performed.

**Next: Phase 4 — account lifecycle and durable recovery.** Replace the appropriate integration gates only when canonical operations, current actor/target transaction checks, idempotency, provider recovery and audit are ready. Profile/application parity and business integration have their own later gates. Do not enable a whole legacy feature merely because its actor resolver now reads Rust.

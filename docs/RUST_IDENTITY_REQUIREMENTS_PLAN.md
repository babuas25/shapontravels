# Rust account and agency identity — requirements and phased delivery

Date: 2026-09-16

**Status:** Phases 0–8 complete as staged/isolated-preview implementation and verification. Phases 6–8 add canonical wallet/business integration, existing dashboard/flight route integration, account operation UI, read-only setup checks, failure coverage and build/browser evidence. Verified with 88 ordinary Rust tests, 14 full isolated matrices plus final expanded business/booking and additive upgrade checks, 17 Next/browser suites, TypeScript/ESLint, rustfmt/strict Clippy and an isolated production build. [Phase 6–8 evidence](evidence/RUST_IDENTITY_PHASE6_8_2026-09-17.md), [runbook](RUST_IDENTITY_RUNBOOK.md), [dependency inventory](RUST_IDENTITY_DEPENDENCY_INVENTORY.md).

**Phase 9: localhost authority cutover completed on 2026-09-17.** The user subsequently authorized local activation, selected Jewel Babu as bootstrap Super Admin and instructed that the existing unresolved booking need not be resolved for this cutover. `localhost:3000` now uses pinned Rust canonical identity and wallet authority on a fully retained, separately restored local database. Clerk login remains. Target backup/restore, explicit subject mapping, writer shutdown, activation and signed-in browser checks passed. [Local cutover evidence](evidence/RUST_IDENTITY_LOCAL_CUTOVER_2026-09-17.md). Production activation and external email/webhook delivery remain outstanding; **the overall deployment project is not marked complete**. No real account was deleted and no production authority was changed.

## 1. Outcome and scope

Keep the existing Next.js screens and Clerk login. Move application users, roles, access state, agencies, memberships, account security, profiles and B2B onboarding to the Rust application's PostgreSQL database. Account management and the Rust wallet must work without Supabase identity tables or RPCs.

The user has authorized writing this plan and implementing it phase by phase. Routine isolated implementation and tests do not need repeated approval. This does **not** authorize deleting real Clerk users, clearing a populated database, importing old financial data, sending invitations to real people, or issuing tickets as a test.

### Decisions

1. Clerk owns authentication: credentials, verified authentication identities, sessions and authentication recovery.
2. Rust PostgreSQL owns application authorization: role, access status, agency ownership/membership and application permissions.
3. Next verifies the Clerk session and forwards the verified subject through an authenticated, server-only bridge. Rust loads current authority from its own database; a browser role, owner ID or preview cookie cannot grant permissions.
4. No automatic import of Supabase users, profiles, wallet balances, holds or ledger history. New registry records are created through explicit bootstrap/onboarding workflows. Existing Clerk logins may be retained; a retained login does not automatically regain privileged roles from metadata.
5. Existing Rust bookings, wallets, API clients and stable owner references must not be reset or reassigned. If present, these require an explicit matching review before that identity is activated in the new registry.
6. Existing UI fields, role choices, filters, forms and actions remain. Pending and failed operation states become visible where external-provider work is not yet confirmed.
7. This plan supersedes the earlier wallet plan's decision to retain the Supabase agency directory **only when the identity cutover gate passes**. Until then, existing contracts remain in force.

### Not in this project

- Replacing Clerk's password/session implementation.
- Rewriting the frontend in Rust.
- Migrating every Supabase feature in the site. Marketing content, unrelated supplier configuration and other non-identity features require their own inventory before any claim that Supabase can be entirely removed.
- Importing legacy financial history or completing unrelated wallet refund/reissue/void phases.
- Changing Search → Reprice → Book → NewTicket or enabling an automatic PNR dependency.

## 2. Current source review and concrete failures

| Area | Current behavior | Required change |
| --- | --- | --- |
| Login | Clerk | Retain |
| Dashboard role and active state | Clerk metadata; development preview may change dashboard presentation | Current Rust role/status for authority; preview cannot affect mutations |
| User directory | Clerk roster plus Supabase `app_users` mirror | Rust directory, with Clerk identity/display hydration as needed |
| Agency ownership | Supabase `agencies`; `app_users.agency_code` | Rust agency and membership records |
| Account lock/audit | Supabase RPC and `security_audit_events` | Rust transaction protection and durable operation/audit records |
| Create/delete | Next server actions call Clerk, then update database | Durable Rust-coordinated provider workflow with recovery |
| Sub users | Supabase membership plus Clerk roles; B2B owner can remove own sub user | Preserve scoped behavior; Rust enforces it |
| Profiles/company/staff | Supabase profile/staff/application rows; Cloudinary assets | Rust data and asset references; retain asset provider |
| B2B application approval | Separate role/provider/database writes | Atomic local grant/agency/profile decision with tracked provider work |
| API client administration | Fresh Clerk role; linked Rust API client | Rust role/status plus explicit API-client link/revocation |
| Wallet owner | Supabase agency directory even in Rust wallet mode | Canonical Rust agency key |
| Email/SMS recipients | Some readers scan Clerk metadata or Supabase | Rust authorized recipient directory, existing delivery transports |
| Rate limits | Supabase RPC; local in-memory fallback | Shared Rust store for migrated account mutations |
| Clerk webhook | Signature-verified email relay and welcome handling | Preserve mail behavior; add durable identity event processing separately |

Observed locally: `NEXT_PUBLIC_SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY` are empty. The old lock helper mapped store unavailability to contention. That message handling has been corrected, but it did not migrate storage. Recreating an account does not repair this missing dependency.

### Reviewed entry points

Frontend repository: `../shopontravels` relative to the Rust repository root.

- `lib/dashboard/session.ts`, `lib/account-access.ts`, `proxy.ts`, `lib/roles.ts`.
- `lib/db/users.ts`, `lib/db/agencies.ts`, `lib/db/sub-users.ts`, `lib/db/security.ts`, `lib/rate-limit.ts`.
- `lib/db/profiles.ts`, `lib/db/staff.ts`, `lib/db/upgrade-requests.ts`, `lib/db/document-uploads.ts`.
- `app/(dashboard)/dashboard/users/{page.tsx,actions.ts}` and `agency-users/{page.tsx,actions.ts}`.
- `lib/dashboard/sub-user-guard.ts`, `lib/dashboard/user-roster.ts`, company/profile pages and actions.
- `lib/api-management/server.ts`, `app/api/api-management/route.ts`.
- `lib/wallet/{rust.server.ts,rust-settings.server.ts,payment-options.server.ts,notification-plan.server.ts}`.
- `lib/rust-holds/server.ts`, `lib/rust-passengers/server.ts`, `lib/email/notifications.ts`.
- `app/api/webhooks/clerk-email/route.ts` and existing account-security regression scripts.

Rust: `src/auth.rs`, `src/api_management.rs`, `src/portal.rs`, `src/portal_holds.rs`, `src/passengers.rs`, `src/wallet/portal.rs`, migrations and isolated PostgreSQL tests.

This is a local source review, not a live audit of all Clerk users, deployed environments or historical balances.

## 3. Requirements

### ID-01 — Stable identity and fresh setup

- Use immutable internal identity IDs with a unique Clerk user ID mapping. Email/name are display/contact attributes, never financial ownership keys.
- Preserve a deleted identity tombstone. Re-registration with the same email and a new Clerk ID creates a different identity and agency; it cannot inherit the deleted user's wallet, permissions or bookings.
- Bootstrap the first Super Admin through an explicit, authenticated operator workflow. Validate the selected existing Clerk user; require an empty registry and serialize competing bootstrap attempts. Record the bootstrap actor and target.
- Never grant Super Admin to the first browser login, a supplied email, unsigned webhook or unchecked Clerk metadata.
- Subsequent signups receive the least-privileged applicant/customer state. B2C-disabled behavior remains: they can complete the B2B application but cannot use business features before approval.
- A missing registry row, missing agency, database outage and a deleted identity are distinct states, not silent defaults to an active customer.

### ID-02 — Canonical permissions and sessions

- Preserve roles: `superadmin`, `admin`, `staff_support`, `staff_account`, `staff_media`, `b2b`, `b2b_sub`, `customer`.
- Resolve role and active status from Rust on protected reads and mutations. Respect Clerk authentication failure/bans as an additional denial; Clerk metadata cannot re-enable a Rust suspension.
- Check the current actor and target inside the mutation transaction. Compare expected versions for stale forms.
- Role changes, deactivation and deletion increment authorization version and revoke affected application/API access. Provider session revocation is tracked and retried if needed; database suspension takes effect without waiting for Clerk.
- Revalidate authority before a new external business dispatch. Operations already dispatched must settle/reconcile through their existing state machines; never hide a supplier outcome because the actor was later suspended.
- Administrative bridge credentials are distinct from public machine tokens and remain server-only. Restrict the bridge to dedicated identity capabilities before production; generic admin authentication alone is not proof of an end user's authority.

### ID-03 — Role and management parity

| Actor | Allowed account operations | Boundaries |
| --- | --- | --- |
| Super Admin | Create/invite, assign all roles, activate/deactivate, manage profiles, delete | No self role/access/delete action; cannot remove the last active Super Admin |
| Admin | Create/invite and manage permitted accounts, including Admin role, profiles and B2B applications | Cannot grant/manage Super Admin; cannot delete global accounts |
| B2B owner | Invite/revoke invitation, rename, suspend/reactivate and remove own sub users; permitted staff records | Must be stored agency owner; cannot select arbitrary role/agency or affect another agency |
| B2B sub user | Own permitted profile/staff record and assigned business features | Cannot manage peer users or agency ownership |
| Other staff/customer | Existing feature-specific and self-profile permissions | No global account management |

Current `removeSubUser` behavior is a scoped exception to global Super-Admin-only deletion. Preserve it deliberately and test it; do not accidentally remove that existing functionality.

### ID-04 — Agency and membership integrity

- Create exactly one fresh agency for each newly approved/provisioned B2B owner; preserve the `ST-B2B` + six-digit display-code format.
- Stable agency ID/code must be unique and immutable. Concurrent provisioning must return the existing agency or a typed conflict, never duplicate agencies.
- One active agency membership per user. Each active agency has exactly one owner. Ownership is a stored relationship, not inferred solely from `b2b` role.
- Sub-user grants require an existing active agency. New B2B owners do not select somebody else's agency.
- Role changes cannot leave orphaned memberships, owners or sub users. Owner transfer is an explicit audited workflow if needed; until implemented, block incompatible changes with a clear dependency reason.
- Suspended/deleted owners must not leave unrestricted active sub-user access. Define agency suspension and owned-client revocation in the same local transaction.
- Returning an empty list on a failed membership read is forbidden; surface service unavailability.

### ID-05 — Account provider workflows and recovery

Each operation has a UUID, actor, target/subject, request fingerprint, expected version, action, stage, timestamps and safe failure code. Identical request replay returns the original operation; changed payload under the same ID is a conflict.

Creation:

1. Authenticate/authorize; validate input and invitation/member limits.
2. Persist a prepared operation and audit before calling Clerk.
3. Execute the necessary provider call through the trusted server adapter.
4. Persist confirmed Clerk identity and application user/agency changes.
5. Report complete only after required local writes are committed. Queue optional welcome/email work separately.

Passwords are sent only to Clerk through the authorized adapter and are never persisted in operation payloads, fingerprints, logs, audit events or retry queues. If an unattempted create needs a password again, request re-entry. After an uncertain create call, reconcile the provider outcome before permitting another create.

Provider state: `prepared → dispatching → provider_confirmed → completed`; failures before dispatch can be retryable, while uncertain dispatch becomes `needs_reconciliation`. Worker lease expiry is not evidence that Clerk performed no write. Use provider correlation metadata and verified reads where supported; never assume Clerk supports arbitrary idempotency keys.

For role/status changes, local authority is decisive. Mirror metadata and revoke provider sessions as tracked effects; a delayed metadata event cannot undo a newer Rust decision.

### ID-06 — Deletion and re-registration

- Perform a dry-run dependency check and show the affected account/agency and any blocker before confirmation in the existing UI.
- Refuse self-deletion and deletion/demotion/suspension of the last active Super Admin, including concurrent requests.
- Preserve financial and booking history. Identity deletion must not cascade into wallet, ledger, audit, bookings, ticket evidence or reservations.
- Block owner deletion while active sub users, an unresolved provider operation, funds/holds, pending financial requests or unresolved bookings require resolution. Allow suspension while resolving blockers.
- Before provider deletion, durably deny new application access and suspend linked API credentials/sessions. Uncertain provider deletion stays visible and recoverable.
- After confirmed provider deletion, retain a non-login tombstone and immutable ownership/history references. Apply explicit PII retention/redaction policy separately; “fresh setup” is not a blanket purge instruction.
- Re-registering the same email never reconnects the old agency/client/wallet automatically.

### ID-07 — Invitations and webhook events

- Store invitation intent, intended role/agency, issuer and state in Rust. Associate acceptance with a verified provider invitation/identity, not browser metadata.
- Preserve invitation send/revoke, existing email templates and redirect behavior. Protect owner scope and allowed grant roles on create, accept and revoke.
- Use a signature-verified Next webhook boundary, bounded payloads, and a durable Rust inbox keyed by provider event ID.
- Duplicate events return success after durable acceptance; out-of-order or stale events cannot restore privileges or resurrect tombstones.
- Provider deletion observed outside the app disables the mapped local identity and linked access. A provider role-metadata update is not an application role grant.
- Failed durable acceptance returns a retryable error. Inbox workers expose retry/unknown outcomes and dead-letter/reconciliation state.

### ID-08 — Profiles, company, staff and B2B approval

- Preserve existing personal/company/bank/contact/profile fields, field validation, staff data, company logos and business-document upload/removal.
- Keep Cloudinary/storage transport; store validated object references in Rust. A user-supplied URL cannot confer ownership or signed-read permission.
- Enforce field-level rules server-side, including agency identity first-submit locks and existing manager overrides.
- Keep absent data distinct from failed reads so an outage never replaces populated forms with writable blanks.
- Add expected-version checks for edits; retain explicit removals instead of reconstructing intentionally cleared data from old applications.
- B2B application submit, resubmit, pending review, accept and reject preserve existing behavior. Approval atomically records decision, B2B role, agency/membership and profile transfer; provider/email effects are tracked separately.
- Protect profile/document reads with the same target-role and agency boundaries as writes. Agency branding resolves through its canonical owner.

### ID-09 — Wallet, bookings, passengers and API clients

- All owner/assignee/receiver resolution uses the Rust directory, including finance receiver lists, staff-on-behalf bookings and notifications.
- Bridge payload role/owner is cross-checked or replaced by database-derived authority in Rust routes. Changing only the Next session helper is insufficient.
- A new B2B agency's wallet is explicitly provisioned with zero balance. Account creation never grants credit, fake deposits, supplier ticketing rights or machine API permissions.
- A retained Rust financial owner may be attached only to the same verified identity mapping. No display-name/email matching, auto-relinking or new agency code for an existing funded owner.
- Machine scopes and portal roles remain separate. B2B creation does not silently create/enable an external API client.
- Disabling/deleting an identity revokes linked client access according to the existing client model, while reconciliation of existing business operations remains possible.
- Validate B2B deposit request, admin review, shared sub-user wallet access, statement and receipt branding using isolated synthetic funds.

### ID-10 — Concurrency, audit and operational correctness

- Use database transaction locks for local invariants; never hold a database transaction open across Clerk/email/asset network calls.
- Durable operation leases include owner token, expiry and fencing/version checks. An expired/stale worker cannot finalize a newer attempt.
- Serialize last-Super-Admin checks with corresponding role/status mutations. A process-local mutex or expiring lease alone is insufficient.
- Append immutable security audit in the same transaction as local state changes. Provider attempt records precede external writes; completion records reference the same operation.
- Audit stores safe actor/action/target/decision identifiers, versions and allowlisted metadata, not credentials, session tokens, raw profile documents or password-bearing requests.
- Rate limit account actions in a shared store, preserving existing action limits. No production in-memory fallback if the authority store is unavailable.
- Typed errors: unauthenticated, forbidden, not found, version conflict, operation busy, security store unavailable, provider outcome unknown and deletion blocked. Do not label every failure “another change in progress.”
- Support bounded pagination/cursors, indexed lookups, body limits, request timeouts, structured non-sensitive logs, recovery metrics and service readiness checks.

## 4. Target data model

Table names are implementation contracts to be finalized in the migration phase; add tables incrementally, not unused placeholder schemas.

| Entity | Required data and constraints |
| --- | --- |
| `portal_users` | Internal UUID, unique Clerk ID, role, status, authorization version, contact/display snapshot, timestamps, deletion tombstone |
| `portal_agencies` | Internal UUID, unique immutable code, status, version, timestamps |
| `portal_agency_memberships` | User/agency IDs, owner/sub role, unique current membership and one owner per agency; restrictive FKs |
| `portal_identity_audit` | Immutable operation/actor/action/target/outcome/version snapshots, bounded metadata |
| `portal_identity_operations` | Idempotency key/fingerprint, actor/target, operation type/state, version, safe result/error, provider correlation and fenced lease |
| `portal_identity_effects` | Transactional outbox for metadata/session/client synchronization and notifications; deduplication and retry state |
| `portal_identity_events` | Verified provider event inbox, unique event ID, processing/version state and bounded retained payload |
| `portal_identity_invitations` | Intent, provider ID, normalized destination hash/contact, intended grant/agency, state, expiry, acceptance mapping |
| `portal_user_profiles`, `portal_staff_profiles` | Existing field parity, owner ID, version, document metadata and timestamps |
| `portal_b2b_applications` | Applicant, validated fields/documents, state, reviewer, decision/version and timestamps |
| Bootstrap/activation state | One-time bootstrap marker, canonical authority mode/version and activation audit |

Reuse the current Rust shared rate-limit store where compatible. Do not duplicate financial tables or store Clerk passwords. Database roles used by the service must not let public/browser clients access these tables directly.

## 5. API and frontend contracts

The table below preserves the initial conceptual contracts. Phases 2–8 implement staged session/bootstrap/onboarding, lifecycle operations, directory/profile/application/events and business integration; see [the current API contract](PORTAL_IDENTITY_API.md). OpenAPI and executable tests describe the implemented route names, which may differ from these initial contracts.

| Contract | Purpose / permissions |
| --- | --- |
| `POST /admin/portal-identity/session` | Trusted bridge, verified Clerk subject; resolve existing canonical identity/capabilities/membership; read-only |
| `POST /admin/portal-identity/onboard` | Staged least-privilege registration; pending create/invitation reservations block competing onboarding |
| `POST /admin/portal-identity/bootstrap` | Staged one-time operator-only verified selection; empty registry and immutable bootstrap marker |
| `POST /admin/portal-identity/users/query` | Authorized roster, filters, counts, target detail and cursor pagination |
| `POST /admin/portal-identity/operations` | Implemented role/access/agency commands; actor and scope resolved by Rust; dedicated deletion routes below |
| `POST /admin/portal-identity/operations/query` | Authorized status and safe replay/result lookup |
| `POST /admin/portal-identity/creates/{prepare,dispatch,query,finalize,cancel}` | Staged durable account creation; production writer disabled |
| `POST /admin/portal-identity/invitations/{prepare,dispatch,query,revoke,accept}` | Staged invitation lifecycle; production invitation adapter disabled |
| `POST /admin/portal-identity/deletions/{preview,prepare,dispatch,query,finalize}` | Staged dependency preview and deny-first deletion; production writer disabled; financial/business references block pending integration |
| Internal reconciliation methods | Trusted read-only provider evidence with fencing; no public force-complete endpoint |
| `POST /admin/portal-identity/agencies/query` | Scoped agency/owner/member/assignee reads |
| `POST /admin/portal-identity/profile` | Typed read/update/document-reference commands with field policy/version checks |
| `POST /admin/portal-identity/applications` | Typed submit/read/review commands |
| `POST /admin/portal-identity/events` | Restricted, signature-verified provider event ingestion through Next |
| Worker claim/finish endpoints or internal worker methods | Restricted outbox/inbox execution, bounded batches and fencing tokens |

Public client balance/statement endpoints remain the wallet API. Do not expose the admin identity bridge, a generic SQL/RPC proxy or arbitrary role-setting endpoint to B2B API clients.

Frontend session model should distinguish `authenticated`, `onboarding_required`, `suspended`, `deleted`, and `unavailable`. A missing agency/setup issue should show a useful setup/service message instead of an unrelated 404.

Split low-level backend transport from actor resolution before adding identity reads, to avoid `identity → apiActor → identity` circular imports. Use one canonical server-side identity resolver throughout the portal. Presentation preview stays separate from the mutation actor.

## 6. Delivery phases and gates

No phase is complete just because its code compiles. Update this table with files, test commands, evidence and remaining limitations. Do not mark the entire project complete while deployment/configuration or required browser journeys remain.

| Phase | Deliverables | Exit evidence | Current status |
| --- | --- | --- | --- |
| 0 — Requirements and inventory | This plan; authority matrix; dependency map; fresh-data and preserved-ID policy | Source references and every existing account action mapped | Complete |
| 1 — Database and domain foundation | Additive migrations; Rust role/status/policy types; user/agency/membership invariants; immutable audit | Fresh disposable PostgreSQL migration and concurrency/integrity tests; no live activation | Complete, staged; [evidence](evidence/RUST_IDENTITY_FOUNDATION_2026-09-16.md) |
| 2 — Identity API, bridge and bootstrap | Server-only authenticated routes, low-level transport split, one-time verified operator bootstrap, least-privilege onboarding, typed errors | Unauthenticated/machine/forged-role rejection; concurrent bootstrap; duplicate onboarding | Complete, staged; [evidence](evidence/RUST_IDENTITY_BRIDGE_2026-09-17.md) |
| 3 — Canonical session authority | Rust session resolver; protect middleware/actions/APIs; remove authority from preview/Clerk metadata in switched paths | Stale metadata/forged preview cannot grant access; unavailable database fails closed | Complete, isolated preview; unported feature paths gated; [evidence](evidence/RUST_IDENTITY_SESSION_2026-09-17.md) |
| 4 — Account lifecycle and recovery | Create/invite/role/status/delete operations; provider adapter; durable outbox/inbox; audit/rate limits; API revocation | Partial failure/crash/replay tests; self/last-admin/dependency guards; delete/re-register isolation | Complete, staged/isolated: lifecycle, official adapters, signed inbox, fenced workers/mail, recovery UI and deletion barriers; [completion evidence](evidence/RUST_IDENTITY_PHASE4_COMPLETION_2026-09-17.md) |
| 5 — Agency, sub users, profiles and approval | Full company/staff/profile/document parity; invitations; B2B application review; canonical branding | Two-agency isolation, field locks, concurrent approval, profile/version/document permissions | Complete, staged/isolated: profile/staff/documents/logo, atomic application workflow, owner rename, adapters and browser journeys; [completion evidence](evidence/RUST_IDENTITY_PHASE5_COMPLETION_2026-09-17.md) |
| 6 — Business integration | Wallet owner/receivers; bookings/assignees/passengers; API clients; recipient directory; Rust-side actor revalidation | Identity through deposit/approval/statement and booking ownership; no identity Supabase access | Complete — isolated/staged evidence |
| 7 — UI parity and local developer flow | Existing UI adapters, operation states, setup errors, configuration checks, safe bootstrap/run docs | Local browser user/admin/B2B/sub-user journeys; no irrelevant 404 or false success | Complete — ported staged UI and fixture journeys |
| 8 — End-to-end and failure review | Full acceptance matrix, dependency scans, fault injection, clean build/static checks, evidence record | All mandatory tests below; real configuration prerequisites listed separately | Complete — local/synthetic verification; target prerequisites separate |
| 9 — Controlled activation and completion | Backup/restore rehearsal, identity-only cutover, bootstrap selected operator, smoke checks, monitoring/runbook | Actual target configuration verified; no mixed authority; rollout state recorded | Localhost cutover complete — pinned canonical marker active, retained data and signed-in browser verified. External delivery and production activation pending |

### Phase dependencies

`0 → 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9`

Use isolated databases and synthetic Clerk adapters for development. Provider writes in verification must be fake or an explicitly designated disposable provider environment. No real invitation/email/account deletion is necessary to prove local state-machine correctness.

### Phase 1 detail

- Add the minimal registry/agency/membership/audit schema first, with strict IDs, role/status checks, versions, unique relationships and restrictive deletes.
- Centralize policy definitions and serialization instead of repeating role strings across handlers.
- Establish transaction helpers for stable authority reads and serialized identity mutations. Do not expose incomplete lifecycle writes to the frontend.
- Prove rollback of local state when audit fails, foreign-key/uniqueness protection, deleted identity non-reuse, and database-generated timestamps.

### Phases 2–4 detail

- Bootstrap uses an explicit selected Clerk ID and confirmed provider lookup. Fresh environment only; existing registry restoration uses a distinct audited recovery path.
- Define a deployment mode and activation marker that prevent half-switched writers. New routes are staged until their dependencies are ready.
- Add a strict provider interface and fake adapter before connecting Clerk writes. Preserve existing provider error sanitization.
- Implement one action at a time: onboarding/create, role, status, invite/revoke, deletion; each gets its failure/replay tests before the next.
- Resolve outstanding prepared/unknown operations before authority cutover. Old workers/webhooks cannot continue mutating canonical roles afterward.

### Phases 5–7 detail

- Replace helper contracts systematically; do not globally substitute `supabaseAdmin()` with another client.
- Update direct Clerk-metadata readers as well as shared helpers: roster, application reviewers, deposit receivers, notification recipients, API-client targets, staff assignees and sub-user guards.
- Preserve UI counts, search/filter/sort, invitations, profile dialogs, uploads, company branding and server-rendered pages.
- Update existing test fixtures to return canonical identities rather than silently recreating metadata authority.
- Local startup validates Rust connectivity, migration readiness, identity activation and bootstrap status without printing secrets. Keep `.env.example` and setup docs aligned; do not silently rewrite the user's `.env.local`.

## 7. Mandatory acceptance matrix

| Test group | Cases |
| --- | --- |
| Bootstrap | Explicit known Clerk user; duplicate/concurrent bootstrap; missing/invalid provider identity; never automatic first-login elevation |
| Authentication | Signed out, invalid/revoked session, banned user, suspended/deleted Rust user, missing registry, database unavailable |
| Authorization | Each role; cross-agency read/write; forged actor/target/owner; stale Clerk metadata; development preview; ordinary admin vs Super Admin |
| Concurrency | Last-admin demote/delete/suspend races; stale versions; owner provisioning races; membership reassignment; audit rollback |
| Create/invite | Duplicate submission; same ID/different payload; existing email; provider success/local failure; request timeout; duplicate acceptance; revoked/expired invitation |
| Provider recovery | Crash before dispatch, after dispatch, before result commit; expired worker; late completion; duplicate/out-of-order webhook; reconciliation does not duplicate users |
| Deletion | Self/last-admin forbidden; B2B owner blockers; scoped sub-user removal; client revocation; retained history; uncertain provider delete; same-email re-registration isolation |
| Profiles | Admin/owner/sub-user permissions; field locks; manager override; stale save; explicit removal; asset ownership; failed read cannot overwrite data |
| B2B approval | Pending/accepted/rejected; concurrent reviewer; agency/profile atomicity; repeat approval; failed notification does not revert approval |
| Business | New agency zero wallet; shared sub-user owner; deposit request/review; statement; staff-on-behalf owner; suspended API token; saved passengers/booking visibility |
| Isolation | Supabase identity calls forbidden in switched mode; no automatic legacy fallback; unrelated legacy features remain explicitly inventoried |
| Operations/UI | Busy vs unavailable vs unknown errors; durable status after refresh; cursor pagination; failed job visibility; no PII/secrets in logs |

Minimum tools: Rust unit tests, disposable PostgreSQL integration tests through real routers, offline Next bridge/action regressions, TypeScript typecheck, targeted lint, Rust fmt/Clippy, relevant existing wallet/booking tests, and browser journeys against synthetic local services.

## 8. Cutover, fresh-data handling and recovery

1. Finish all previous gates and record tested revisions/schema versions. Back up the target Rust database and prove restoration on a separate disposable database.
2. Inventory retained Clerk IDs and existing Rust owners/clients/bookings without writing. Select the operator ID for bootstrap and explicitly map any retained business identities. Do not use email as the mapping key.
3. Pause account mutations and old identity workers; reconcile pending operations and verify no request is still dispatching a provider mutation.
4. Apply additive migrations, configure server-only bridge/provider credentials, bootstrap the selected operator, and perform canonical read checks.
5. Activate the Rust identity authority consistently in frontend/backend; enable mutation routes only when schema, bridge and bootstrap checks pass.
6. Smoke-test the actual target with non-destructive reads; use designated disposable accounts for any approved write verification.
7. Observe authorization failures, unknown provider operations, webhook backlog and audit failures. Keep failed state visible and access fail-closed.

Local retained-booking exception: an initial loopback `_identity_test` activation may explicitly acknowledge old `outcome_unknown` bookings when every identity/financial/provider queue and mapping issue is clear. The mapping digest binds booking state and update time; activation locks and rechecks the reviewed booking rows and rejects pending/recent outcomes. This does not resolve a booking, waive account-deletion dependencies or permit the exception on a production database. The operator evidence records the acknowledgment.

**Rollback:** Before the first new-authority write, return to the old mode only if its authority data/configuration is valid. After a Rust role/membership/account change, toggling back to stale Clerk/Supabase authority can restore revoked access; do not do that. Pause mutations and recover forward, or restore a coordinated reviewed snapshot/provider state. Never downgrade by dropping identity tables or restoring only one side of a provider operation.

Fresh financial data remains a separate choice from authentication identity. This implementation does not delete all Clerk accounts or erase existing Rust transactions. No credentials need to be pasted into chat; missing target credentials belong in the local/deployment secret configuration.

## 9. Completion definition

The project is complete only when:

- Account, agency, role, status, profile, invitation and B2B approval flows function with Supabase identity configuration absent.
- Clerk still provides login; current Rust state controls application permissions across all switched surfaces.
- B2B creation produces a usable agency and explicit zero-balance wallet onboarding; deposit requests and existing UI work.
- Provider failures cannot duplicate identities, restore revoked access or lose operation evidence.
- Account deletion/re-registration preserves ownership boundaries and financial history.
- All required acceptance tests and local browser journeys pass, remaining external prerequisites are resolved, and activation evidence is recorded.
- The phase table, OpenAPI, setup/runbook, `.env.example` and dependency inventory match actual behavior.

## 10. Execution log

### Localhost authority activated — 2026-09-17

- User authorized localhost cutover and selected Jewel Babu; the existing unknown booking remains unresolved by request.
- Retained the original migration-33 database and restored its full contents into `portal_local_20260917_identity_test`; applied additive migrations through 0046. Verified all original business rows and sequences, then backed up/restored all 79 target tables.
- Reviewed exact Clerk subjects: preserved six active accounts and their existing roles, three agencies with zero BDT wallets, and existing client/booking/draft ownership. Two absent provider subjects were represented by local denied tombstones to retain historical client references. No provider account was deleted/created, no metadata was changed and no financial history was imported.
- Stopped the old local Next/Rust writers, rechecked unchanged source fingerprints, activated rollout revision 1 in maintenance and restarted without operator credentials. Frontend identity and wallet modes are `rust-canonical`; the portal launcher now resumes the selected canonical database.
- Actual signed-in browser: Jewel Babu Super Admin; six active accounts; eligible removal review succeeds; booking-linked removal shows `booking_references`; all four bookings remain visible; three wallets show zero balances. No deletion confirmation, email, supplier Book/Issue/Cancel or Cloudinary write was performed.
- Identity mail and inbound event delivery remain disabled; production setup is separate. See [local activation evidence and restart instructions](evidence/RUST_IDENTITY_LOCAL_CUTOVER_2026-09-17.md). Older execution entries below describe their state at the time and do not override this activation.
- Follow-up: fixed removal-review visibility and dependency explanations in the existing Users & Roles component. The signed-in browser verified loading, blocked/eligible previews, visible errors and keyboard dismissal without account deletion. Local rollout resumed at revision 3 after a verified backup/restore and fresh preflight; the unknown booking remains unchanged.

### Phase 0 — 2026-09-16

- Reviewed both repositories and traced the account error to the existing Supabase security dependency.
- Expanded the migration scope to include all current identity consumers; documented the global deletion and scoped sub-user deletion distinction.
- Defined fresh-registry bootstrap, existing-ID preservation, provider recovery, no automatic financial relinking and activation gates.
- No real accounts, financial data, runtime authority flags or environment credentials changed by this planning phase.

### Phase 1 — 2026-09-16

- Added migration `0034_portal_identity_foundation.sql`: users, agencies, memberships and immutable identity audit. Tables start empty; no data import or activation occurs.
- Added `src/identity.rs`: strict role/status types, global management and scoped sub-user policies, transaction-level authority serialization, last-active-Super-Admin guard and typed audit writer.
- Added `tests/identity.rs`: real PostgreSQL coverage for deferred ownership/membership constraints, concurrent admin demotions, contention, rollback, immutable keys/audit and same-email re-registration isolation.
- No identity routes are exposed yet. Bootstrap, Clerk integration, provider recovery and frontend cutover remain subsequent phases, not partial live functionality.
- Full evidence and verification commands are in the [foundation evidence record](evidence/RUST_IDENTITY_FOUNDATION_2026-09-16.md).


### Phase 2 — 2026-09-17

- Re-read Phase 1 evidence and verified current source and the existing foundation tests on another fresh disposable database before implementation.
- Added migration `0035_portal_identity_staging.sql`, dedicated bridge/operator credentials, a read-only Clerk adapter and three staged identity routes. Default mode remains disabled; no canonical/live activation value exists.
- Added serialized, verified operator bootstrap with atomic registry/marker/audit; least-privilege onboarding is idempotent and preserves suspension/tombstones. Known retained business references require explicit matching review.
- Split Next low-level transport from actor resolution and added a server-only staged identity adapter with fresh Clerk session checks. Existing dashboard and mutation authority is unchanged.
- Added isolated real-router PostgreSQL tests, fake-provider tests and offline Next bridge regressions, plus an isolated bootstrap command and configuration/API documentation.
- No real Clerk account/provider write, live database migration, environment secret change, wallet reset or cutover was performed. Phase 3 is the next implementation phase; full completion remains gated through Phase 9.
- Full results and limitations are in the [Phase 2 evidence record](evidence/RUST_IDENTITY_BRIDGE_2026-09-17.md).


### Phase 3 — 2026-09-17

- Added an uncached canonical Rust account resolver and explicit `SHAPON_IDENTITY_AUTHORITY=legacy|rust-preview` selector. Preview requires staged backend connectivity on loopback and rejects production; no runtime configuration was changed.
- Removed the development role cookie from shared account/dashboard authority in all modes. Only DashboardShell presentation can use the preview role; actual actions and protected data use the real actor.
- Added typed current-account status API and setup/status page; canonical session reads use stored role, state, versions, contact and agency/ownership data without legacy mirror/metadata reads.
- Guarded unported lifecycle/business APIs, server actions, actor helpers and generic transport in isolated preview. Old target/directory/provider workflows are not implicitly enabled with a newly canonical actor.
- Added strict response invariants, direct action/API bypass regressions, all-role session/version and owner/agency suspension tests. Existing relevant legacy regressions and static checks passed.
- Phase 4 is next. Existing live account create/delete remains on the old path; browser journeys, business integration and deployment activation remain later gates. See [Phase 3 evidence](evidence/RUST_IDENTITY_SESSION_2026-09-17.md).


### Phase 4, first lifecycle slice — 2026-09-17

- Verified the existing foundation/bootstrap/session implementation against fresh disposable PostgreSQL databases with migration 0036 included.
- Added strict staged role/access operations and authorized operation lookup. Rust reloads current actor/target/scope, enforces expected versions, replay fingerprints, shared action limits and self/last-admin/agency guards.
- Local role/status, owner-agency suspension, dependent member authorization invalidation, linked API/credential/session revocation, immutable audit and durable provider effects commit together. Financial rows and ownership links are retained.
- Added a provider-effect interface with synthetic tests, durable pre-dispatch attempts, per-subject ordering, fencing, bounded proven-not-sent retries, uncertain-outcome reconciliation and stale metadata supersession. No production Clerk write adapter or worker is wired.
- Next preview remains read-only with the existing integration gates. No environment files, live authority, real accounts or real funds were changed.
- **Continue within Phase 4 next:** implement create/provisioning and invitations/revocation/deletion one workflow at a time, agency reactivation/provisioning, provider adapter/worker and signed durable inbox; complete recovery UI/queue pagination and remaining audit/operational visibility. Keep pending effects visible and preserve unknown-outcome blocking. Do not advance to Phase 5 or lift preview gates based on this partial slice.
- See [Phase 4 role/access evidence](evidence/RUST_IDENTITY_OPERATIONS_2026-09-17.md) for executed tests and limitations.


### Phase 4, agency lifecycle slice — 2026-09-17

- Added explicit `provision_agency` for eligible, already registered customers and `reactivate_agency` for a suspended stored B2B owner/agency. These are manager-only staged operations using the existing version/replay/audit/outbox boundary; both actor and target get fresh provider checks at the HTTP boundary.
- Provisioning atomically grants B2B, creates one immutable agency code and owner membership, and creates one fresh BDT wallet with zero balance and a permanent agency-to-wallet binding. Known retained subject references require matching review; existing agency/financial codes are skipped, and no historical wallet is upserted or relinked.
- Reactivation requires the expected user and agency versions, restores the same owner/agency, and invalidates member authority. It preserves individual sub-user suspensions, frozen wallet state, financial history and disabled API credentials. Archived agencies remain terminal.
- Migration 0037, synthetic PostgreSQL agency matrix, prior operation/recovery matrix, foundation and bootstrap/session regression passed. Strict input testing caught and fixed extra-field acceptance on the field-free provisioning command. Full ordinary Rust suite: 82 passed. A separate upgrade of a cloned migration-0036 evidence database preserved all 11 existing identity/operation/audit/wallet table hashes and inferred no financial bindings.
- Shared synthetic identity test helpers moved to `tests/identity_support/mod.rs`; no frontend authority gates or environment files changed. No production provider writer is connected.
- **Next remains Phase 4:** provider account creation with durable prepared intent/password-safe recovery, invitations/revocation/acceptance, deletion/dependency checks, provider adapter/worker and signed durable inbox. Application-review/profile transfer and frontend parity are still later work. This provisioning command does not implement B2B application approval or create a Clerk account.
- Latest [agency lifecycle evidence](evidence/RUST_IDENTITY_AGENCIES_2026-09-17.md); keep earlier evidence and all uncommitted work.


### Phase 4, durable account-create slice — 2026-09-17

- Added migration 0038 and `src/identity/creates.rs`: password-free prepared intent, same-email in-flight reservation, immutable create/attempt evidence, fenced dispatch/reconciliation, five-attempt proven-not-sent limit, safe pre-dispatch cancellation, and atomic local finalization after verified provider correlation.
- Added staged create prepare/dispatch/query/finalize/cancel routes. The production runtime has no create writer; only explicit synthetic adapter injection dispatches in tests. Password-bearing requests are separate non-serializable/non-debug types and never reach operation fingerprints, database payloads, audit or retry storage.
- Finalization checks current manager authority, fresh provider identity, stored agency/version/capacity and retained references before creating a new canonical user. B2B gets a fresh agency/zero-balance BDT wallet; sub-users get only the selected validated agency membership; customer remains onboarding. Existing users and financial owners are never adopted from email/name/provider-result matching.
- Provider success is durably recorded separately from local commit. Audit/local-store failure can retry finalization without another provider create. Unknown delivery/expired leases require read-only correlated reconciliation, cannot be cancelled or recreated under another operation ID for the same email, and never use absence as proof of no write.
- Known pending provider subjects/verified-email reservations block competing automatic onboarding without linking identities. If an uncorrelated subject with no verified email already onboarded, finalization fails closed with matching review instead of silently adopting/promoting it; production provider correlation/webhook coordination remains a wiring gate.
- Synthetic create matrix, existing identity/agency/recovery regressions and ordinary Rust tests passed. An additive upgrade of a cloned migration-0037 database preserved 15 existing identity/audit/financial/rate tables. See [create/recovery evidence](evidence/RUST_IDENTITY_CREATES_2026-09-17.md).
- **Next remains Phase 4:** invitations/revocation/acceptance, deletion dependency preview/workflow, official Clerk create/effect/reconciliation adapters and worker wiring, signed durable inbox, welcome-mail effects and recovery UI/operational visibility. Production account creation and the live Supabase replacement are not enabled; keep all preview/business gates until their later parity/cutover work is complete.


### Phase 4, durable invitation slice — 2026-09-17

- Added migration 0039, immutable invitation intent/attempts and branded-mail delivery intent; staged prepare/dispatch/query/revoke/accept routes and explicit synthetic adapter injection. Environment configuration enables neither invitation writes nor acceptance evidence reads through this new adapter.
- Managers grant allowed roles; a B2B owner's command accepts only an email and derives role/agency/version from current Rust ownership. Preparation and acceptance enforce active actor/issuer/agency state, version and the 100-member limit across memberships, pending creates and invitations. Shared hourly limits and cross-workflow email/UUID reservations prevent bypass through alternate workflows.
- Issue/revoke attempts commit before provider I/O, use fenced leases and bounded proven-not-sent retries, and retain unknown outcomes for read-only reconciliation. Local revoke immediately blocks acceptance and mail, including when an issue/revoke outcome remains unknown. It never falsely reports provider revocation as confirmed.
- Acceptance requires a trusted adapter's exact invitation/operation/provider-subject correlation and verified email, then rechecks issuer/scope inside the final transaction. A concurrent revoke wins before grant commit. Shared provisioning commits a fresh identity, required agency/membership/zero BDT wallet, ordinary provider effects and audit; existing identities/financial references require matching review.
- The synthetic invitation matrix, additive migration-0038 clone upgrade (17 existing table fingerprints), all five previous identity PostgreSQL matrices and the ordinary Rust suite passed. The earlier account-create flow was regression-tested after extracting shared provisioning. See [invitation evidence](evidence/RUST_IDENTITY_INVITATIONS_2026-09-17.md).
- Branded-mail intent is **not delivery**. Existing Next templates/redirects and Clerk login are unchanged; real adapters, expiry handling, signed inbox, mail delivery/reconciliation and UI are still required. No real invitation, account deletion, live database migration or cutover occurred.
- **Continue within Phase 4 next:** deletion dependency preview and durable workflow, then official provider/read-correlation/expiry contracts, signed inbox, provider/mail workers and recovery UI. Keep production writers and all frontend preview/business gates disabled until the remaining parity/activation gates pass. Phase 4 is not complete.


### Phase 4, guarded deletion slice — 2026-09-17

- Added migration 0040, an immutable deletion/attempt journal, irreversible `deleting` status guard and staged preview/prepare/dispatch/query/finalize routes. The runtime has no environment-enabled deletion adapter; tests explicitly inject a synthetic provider.
- Preview returns current target/agency identity, expected version, review token and typed blockers. Preparation rechecks the review and current authority: only Super Admin globally, or the active stored B2B owner for their own sub-user. Self-delete is denied and the serialized last-active-Super-Admin guard remains enforced. Manager delete is 10/hour; owner removal shares the 40/hour access bucket.
- Preparation atomically records intent/audit and denies canonical access before a provider call; an owned agency is suspended. Provider deletion uses durable attempts, leases/fences, bounded proven-not-sent retries and read-only unknown-outcome recovery. Confirmed deletion is saved separately from local finalization, so an audit failure never requires another provider delete.
- Finalization rechecks dependencies and retains the user/subject/PII, agency/membership and all history as a non-login tombstone; eligible owned agencies are archived. Deleted sub-user memberships stay for history but no longer consume live member capacity. Same-email registration cannot reuse the old internal identity.
- **Conservative boundary:** any known wallet (even zero), client, booking/draft or financial actor reference blocks this staged deletion, as do non-deleted sub-users and unresolved provider work. Legacy business writers do not yet participate in the canonical identity barrier. Newly discovered client access is revoked on recovery while the dependency remains blocked. No caller override or purge is provided; resolving these dependencies requires later reviewed business integration, not deleting historical rows.
- The deletion matrix, six prior identity PostgreSQL matrices, ordinary Rust suite and additive upgrade of a migration-0039 evidence clone passed. The upgrade preserved 20 existing table fingerprints and created no deletion intents. See [deletion evidence](evidence/RUST_IDENTITY_DELETIONS_2026-09-17.md).
- No real account/data deletion, invitation, environment secret change, Next authority change or live migration/cutover occurred. The deletion preview/confirmation UI, PII retention policy and production provider semantics remain separate gates.
- **Continue Phase 4 next:** signed durable provider inbox and official provider adapter/correlation/expiry contracts; then bounded workers, mail delivery/recovery, dependency resolution and recovery UI. Keep all frontend/business preview gates and production writers disabled. Phase 4 remains in progress.


### Phase 4 completed — 2026-09-17

- Completed official provider transports, raw-signature Next event boundary, durable inbox/deletion evidence, bounded lifecycle/effect/mail workers, branded recipient/archive delivery, cursor recovery API/UI and denial audit.
- Completed invitation expiry/correlation and signed-delete reconciliation. Uncertain writes/mail are never blindly resent; current scope/version/fence checks remain mandatory.
- Added coordinated business-write barriers and narrowed deletion blockers to unresolved obligations. Zero wallet, settled ledger, client links and historical identity records survive deletion; new terminal-subject dependencies are rejected.
- Verified **86 ordinary Rust tests**, **11 explicit disposable database matrices/upgrade checks**, four offline/browser Next scripts, TypeScript, ESLint and formatting. Upgrade preserved **63 existing table hashes**. See the completion evidence for exact databases, commands and limits.
- Provider writes are disabled by default and live keys cannot enable them. No real account, real mail, live database or cutover was used. Existing user changes and real environment files were preserved. Earlier Phase 4 slice notes above are historical and superseded by this completion entry.
- **Continue with Phase 5 next:** company/staff/profile/document parity, B2B application approval/rejection, field/version permissions and canonical branding. Do not lift remaining business/UI gates or claim production migration complete; Phases 6–9 remain required.


### Phase 5, profile/staff text and branding read slice — 2026-09-17

- Added migration 0043, versioned 23-field profile/six-field staff records, scoped bridge APIs, canonical owner contact branding and a strict server-only Next adapter. Explicit clears remain durable; errors never appear as missing data. Mutation replay requires current authority and preserves newer values.
- Preserved actual source policy: B2B owner/sub-user profiles are self-read-only; managers correct permitted target profiles. Staff records remain self or actual B2B owner only. Agency scope is loaded from Rust, with suspended/deleted access denied.
- Verified 87 ordinary Rust tests, explicit fresh profile/concurrency/failure matrix, additive upgrade with 67 existing table hashes preserved, lifecycle/runtime regression matrices, six offline Next scripts, TypeScript, targeted lint/format. Strict Clippy still reports six existing style warnings in prior runtime code; see [evidence](evidence/RUST_IDENTITY_PROFILES_2026-09-17.md).
- **Phase 5 remains in progress. Continue next:** B2B application submit/resubmit/review and atomic decision/profile transfer; private document references/upload/removal/signed reads and canonical logo; remaining sub-user rename parity and browser journeys. Profile text APIs do not implement these workflows. Keep existing frontend/business gates and preserve explicit cleared keys during eventual application transfer.
- No real data deletion, Clerk/SMTP/Cloudinary write, real environment change or live cutover occurred. Existing uncommitted work was preserved.


### Phase 5 completed — 2026-09-17

- Completed migration 0044, required/optional application fields and attachments, immutable revisions, pending review pagination, reject/resubmit and atomic approval with fresh agency/membership/zero wallet, explicit-clear-preserving profile transfer, provider effects, activation mail and audit. Current application/profile/identity versions prevent stale decisions.
- Completed five company document slots, canonical owner logo, authenticated Cloudinary adapter using existing MIME/size/SVG validation, durable one-shot upload intent, receipt/version revalidation, private signed reads after audit, versioned removal and uploader-scoped recovery listing. Unknown uploads are observed rather than resent; retained storage evidence is not physically deleted.
- Added scoped owner sub-user rename with immutable provider name effects, current directory, strict Next routes/adapters and `/identity-phase5` workspace. B2B/company self-read-only and exact staff scope are preserved. Existing Clerk login and production screens are unchanged; remaining screen integration is Phase 7.
- Verified **88 ordinary Rust tests**, **13 explicit disposable database matrices/upgrade checks**, **69 retained table hashes unchanged**, seven offline Next scripts, two real Chrome fixture suites, TypeScript, ESLint, rustfmt and strict Clippy. The six prior style warnings are resolved. See [completion evidence](evidence/RUST_IDENTITY_PHASE5_COMPLETION_2026-09-17.md) for exact databases, commands and operational limits.
- Current Clerk name updates cannot carry an atomic operation marker. Ambiguous name writes remain visible for support review and cannot be blindly resent or declared complete based only on equal names. Activation mail also preserves the established unknown-delivery policy.
- **Continue Phase 6 next:** wallet owner/receiver directories, bookings/assignees/passengers, API clients and recipient integration, all with Rust-side actor revalidation and retained-history boundaries. Do not lift unported business/UI gates or perform live cutover. Earlier Phase 5 slice notes are historical and superseded by this completion entry.


### Phases 6–8 completed locally; Phase 9 rehearsed — 2026-09-17

- Verified prior phases before expanding business integration. Added dedicated Rust actor/owner/target directory and allowlisted dispatch, per-transaction authority checks, user-version-bound search sessions, explicit client/wallet linking, canonical receiver/notification recipients and scoped native booking/passenger reads and writes.
- Integrated existing dashboard URLs and shell, account roster/forms/filters/sorts, separate transient password dispatch, removal review, agency reactivation, durable operation status, profile/security/application workspace and setup errors. Preserved Clerk authentication and presentation-only avatar/sign-in hydration. Unported legacy actions and identity stores remain fail-closed in preview.
- Passed ordinary Rust suite (88), all 14 prior DB matrices, final expanded business/race/worker and canonical booking/failure matrices, additive upgrade with 75 retained table hashes unchanged, 17 Next/browser suites, static checks and isolated production build. Full commands and limits are in the linked evidence.
- Rehearsed backup/restore on synthetic data: all 77 public tables and sequences match; source unchanged. Started the restored schema and passed actual HTTP readiness/setup checks with all external writers disabled. No live provider or supplier write was used for verification.
- **Continue only Phase 9 next:** selected-target mapping/configuration, production activation mechanism review, target backup/restore, coordinated old-writer shutdown/bootstrap and actual authorized cutover/smoke/monitoring. The no-live-cutover constraint means these activation actions were not performed. The whole project is not yet complete. Preserve both uncommitted worktrees and all evidence.

### Phase 9, read-only target review and maintenance preparation — 2026-09-17

- Fixed incomplete readiness coverage: all 14 identity/business work queues now contribute, including reconciling, unknown/blocked mail, document uploads and dead-letter inbox events. Readiness and preflight use a repeatable read, read-only transaction; their HTTP boundary does not write rate-limit or audit rows.
- Added operator-only preflight with selected local Super Admin check, eight retained-mapping issue categories and bounded SHA-256 fingerprints of 11 identity/business mapping tables. No automatic linking, provider lookup, activation or raw personal/credential data is returned.
- Added a read-only CLI that requires explicit origin/operator, creates a new private review artifact, validates its response, detects mapping drift and refuses overwrite. A clean DB report is explicitly not activation approval; external operator/backup/writer/deployment gates remain unresolved.
- Added paired maintenance controls: Next rejects matched routes/actions; Rust rejects business/legacy/machine traffic and pauses identity workers/cleanup, while liveness and authenticated read-only review remain. Defaults are unchanged. This is a restart/drain deployment control, not a durable canonical activation marker or proof that old replicas/external workers are stopped.
- Added migration 0046's durable pinned canonical marker with release/target headers and fenced `activate`/`pause`/`resume` commands. Operator planning binds preflight mapping digest plus independent backup/restore/writer-shutdown/configuration evidence; concurrent commands, stale revisions, changed mappings and old workers fail closed. The synthetic rollout matrix passed. The marker is implemented but no real target was selected or activated.
- Verified 88 ordinary Rust tests, new SELECT-only preflight/maintenance matrix, canonical business regression, actual binary/HTTP CLI rehearsal preserving all 77 tables and sequences, CLI failure/drift/privacy checks, Next authority/maintenance suite, TypeScript/ESLint, rustfmt, strict Clippy and Rust binary build. See the linked Phase 9 evidence.
- Actual environment/operator selection, reviewed target backup/configuration and authorized cutover remain. Canonical mode/marker implementation is now available behind migration 0046 and synthetic rollout evidence, but no real `.env`, provider, supplier, real data or live deployment was changed.

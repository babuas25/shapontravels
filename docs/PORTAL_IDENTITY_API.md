# Staged portal identity API and isolated session preview — Phases 2–5 (staged)

These routes implement the registry/bootstrap bridge and support the isolated Phase 3 session preview below. Clerk login is retained. Default legacy dashboard/account/business authority remains in place; development role preview is now presentation-only. The new tables are not a live identity cutover. No generic admin token, machine token, Clerk metadata role or preview cookie can authorize these routes.

## Configuration and boundaries

Rust defaults to `PORTAL_IDENTITY_MODE=disabled`. Disabled routes return `503 IDENTITY_DISABLED`; no provider or identity database work runs. The only opt-in value is `staged`. Any other value, including `canonical`, fails startup. Migration 0035 permits only a staged authority marker and preserves the first bootstrap record as immutable evidence.

In an isolated staged Rust service, configure:

- `PORTAL_IDENTITY_BRIDGE_TOKEN`: `stib_` followed by 43 base64url characters generated from 32 cryptographically random bytes.
- `PORTAL_IDENTITY_CLERK_SECRET_KEY`: the designated Clerk instance's server-only secret. Default mode performs user lookup only. Explicit disposable-test writer configuration is documented in the Phase 4 runtime section below; live provider keys cannot enable writers.
- For the initial operator command only, `PORTAL_IDENTITY_OPERATOR_TOKEN`: an independent `stio_` token with the same random-byte encoding, and `PORTAL_IDENTITY_OPERATOR_ID`: a stable non-secret operator label (1–128 ASCII letters/digits or `_-.:@`).

The Next bridge uses `SHAPON_API_BASE_URL`, `SHAPON_IDENTITY_MODE=staged` and `SHAPON_IDENTITY_BRIDGE_TOKEN` matching the Rust bridge token. It does not receive the operator credential or use generic API Management admin credentials. No variable is public. Example files remain disabled; existing `.env`/`.env.local` files have not been modified.

The low-level Next transport is `lib/rust-api/transport.ts`; it has no actor/session imports. Generic business calls are gated in identity preview; the dedicated identity transport uses only its base-URL configuration. Existing API Management exports remain compatible. `lib/identity/server.ts` obtains the authenticated Clerk subject and checks the current provider session ID, user ID, active status and expiration. It accepts no subject/role arguments. Rust independently checks the current provider user's existence, ban and lock state. Provider role/agency metadata is ignored.

The provider lookup contract follows the [Clerk Backend API](https://clerk.com/docs/reference/backend-api) and [Backend User fields](https://clerk.com/docs/reference/backend/types/backend-user). Verified primary email/name snapshots are contact/display only, never matching or ownership keys. Missing/unverified email is allowed and stored as absent.

## Implemented routes

All requests use JSON and `Authorization: Bearer ...`. Bootstrap/session/onboard accept only this body:

```json
{"clerk_user_id":"user_SELECTED_CLERK_ID"}
```

Unknown fields, including role, owner, agency, password or operator ID, are rejected. The subject must match `^user_[A-Za-z0-9_]{1,123}$`.

| POST route | Credential | Behavior |
| --- | --- | --- |
| `/admin/portal-identity/bootstrap` | Separate operator `stio_` | Verify explicitly selected existing provider user, require empty registry, serialize with all registry mutations, atomically insert active Super Admin, immutable bootstrap marker and audit with configured operator ID |
| `/admin/portal-identity/session` | Bridge `stib_` | Require bootstrap marker; return current identity and agency/owner state without onboarding or contact updates |
| `/admin/portal-identity/onboard` | Bridge `stib_` | Require bootstrap marker; atomically create customer in `onboarding` state and audit, or return the existing row unchanged |

Concurrent bootstrap has exactly one winner; subsequent calls return `IDENTITY_BOOTSTRAP_CLOSED`. Onboarding is idempotent by immutable Clerk subject, including concurrent calls. It never upgrades roles or restores suspended/deleted identities. A new Clerk ID with the same email creates a different registry identity.

Bootstrap/onboarding refuse a new mapping when the selected Clerk subject has known retained Rust API-client, staff-client, user-wallet, hold-draft or booking-creator references (`IDENTITY_MATCHING_REVIEW_REQUIRED`). No owner/client/booking is relinked. Legacy agency-code-only ownership still requires the later explicit inventory/matching workflow; this phase does not infer an association from email or name. No agencies, wallets, balances, credits, machine clients, invitations or provider accounts are created by onboarding.

Bootstrap/session/onboard responses have `authority_mode: "staged"`. Their `state` distinguishes `authenticated`, `onboarding_required`, `suspended`, `deleted`, and `setup_required`. Missing registry identity has `user: null`; an onboarding row includes its ID/status. An agency or owner suspension denies its members through `state: "suspended"`. This response is diagnostic/staged and must not be substituted for live authority yet.

`user` contains the immutable internal UUID, Clerk subject, database role/status, row version, authorization version and nullable email/first/last-name snapshots. Agency UUID/code and `is_agency_owner` come from stored membership, never caller input. Callers must check the session state before treating role/membership fields as permission.

## Operator workflow

Use a newly migrated disposable Rust database and a designated disposable Clerk instance for manual development. Automated tests inject a fake provider and do not need Clerk credentials. Applying migrations or starting a staged server does not bootstrap anyone.

With the isolated Rust service configured as above, pass the explicitly selected provider ID to:

```sh
node scripts/identity-bootstrap.mjs user_SELECTED_CLERK_ID
```

The command reads `PORTAL_IDENTITY_MODE`, `PORTAL_IDENTITY_OPERATOR_TOKEN` and `SHAPON_API_BASE_URL` from its process environment; it deliberately does not load `.env`. It permits a loopback HTTP service only. Secrets are neither command-line arguments nor output. After successful bootstrap, remove the operator token/ID from the service configuration and restart. Removing the credential does not erase the marker.

A timeout can happen after a database commit. Inspect the staged session/registry and audit before retrying bootstrap. Never empty an existing registry to reopen bootstrap. Recovery/restoration and live activation are later gates; there is no force-bootstrap/reset endpoint.

## Errors and limits

Errors are safe JSON codes, never raw database/provider responses. Invalid JSON/fields return typed `IDENTITY_INVALID_REQUEST`; oversized bodies return `IDENTITY_PAYLOAD_TOO_LARGE`.

| HTTP/code | Meaning |
| --- | --- |
| 401 `IDENTITY_UNAUTHENTICATED` | Missing/wrong credential or invalid/revoked frontend session |
| 401 `IDENTITY_PROVIDER_NOT_FOUND` | Provider user no longer exists |
| 403 `IDENTITY_PROVIDER_DENIED` | Provider user is banned/locked |
| 409 `IDENTITY_BOOTSTRAP_REQUIRED` / `IDENTITY_BOOTSTRAP_CLOSED` | Explicit setup is needed / bootstrap is permanently closed |
| 409 `IDENTITY_MATCHING_REVIEW_REQUIRED` | Existing business references require a reviewed mapping |
| 409 `IDENTITY_OPERATION_BUSY` | Authority transaction lock timed out |
| 503 `IDENTITY_STORE_UNAVAILABLE` | Database/read/audit write failed; no fallback |
| 503 `IDENTITY_PROVIDER_UNAVAILABLE` | Provider lookup failed, returned invalid data or timed out |
| 503 `IDENTITY_REQUEST_TIMEOUT` | Backend request deadline; inspect state before retrying a write |
| 503 `IDENTITY_DISABLED` | Staged capability not configured |

Identity bodies are capped at 4 KiB, requests at 15 seconds, provider reads at 10 seconds/256 KiB, Next transport at 18 seconds. Shared PostgreSQL rate buckets limit bootstrap to 10/minute and the bridge to 600/minute; authenticated rate-store failures deny access. Rate limits return 429. Role changes add a per-actor 30/hour shared limit; access changes add 40/hour. Exact operation replay does not consume another action allowance. No database transaction is held across provider network I/O.

OpenAPI advertises the implemented routes in the authenticated admin document, with separate bridge/operator schemes. They are excluded from public commercial OpenAPI.

## Deferred work

Phases 2–4 are complete in isolated staged mode. Application/profile parity, full business integration, existing account-management browser journeys and controlled deployment remain Phases 5–9. The runtime below supplies official adapters, signed inbox, workers, mail and recovery UI. Existing live account create/delete still uses its legacy security store until the later cutover.


## Phase 3 isolated canonical session preview

Next additionally supports `SHAPON_IDENTITY_AUTHORITY=legacy` (default) or `rust-preview`. The latter requires a non-production runtime, `SHAPON_IDENTITY_MODE=staged`, and a loopback HTTP `SHAPON_API_BASE_URL`; unknown/production/remote combinations fail closed. The existing backend authority marker remains `staged`. This is not a live cutover setting and was not enabled in the user's environment files.

`lib/identity/authority.ts` is the current-account resolver. It verifies Clerk authentication, reads current Rust identity, validates state/role/membership consistency and returns a typed result without any Supabase or metadata fallback. `getAccountSession()` uses it in preview, retaining internal ID, row/authorization versions, agency relationship and status. Canonical calls are not cached across invocations.

The isolated portal exposes `GET /api/identity/session` and `/identity-status`. The API accepts no target/role input and responds with no-store JSON. Signed-out sessions return 401, suspension/deletion 403, missing registry/agency setup 409, unavailable service/configuration 503. Existing onboarding rows return their customer account state without business permission. The page shows setup/status feedback with reload/sign-out rather than a misleading 404 or an empty editable profile.

Until Phases 4–6 are implemented, the preview redirects dashboard/application/flight pages to status, blocks other APIs/tRPC/old invitation routes, blocks callbacks/scheduler requests and all unsafe-method/Server Action requests. Independent gates in shared business sessions, standalone actor helpers, application actions and generic backend transport also prevent direct invocation from entering old authority paths. Default legacy mode retains the existing webhook/email/business behavior.

The development role cookie is no longer read by `getAccountSession()` or `getDashboardSession()` in **any** mode. `getDashboardPresentationRole()` is only used for DashboardShell navigation; page/action permissions always use the real authority role.

See [Phase 3 evidence](evidence/RUST_IDENTITY_SESSION_2026-09-17.md) for executed tests, coverage and deferred browser/business journeys. Remove each integration gate alongside its complete canonical implementation; do not open all old writers by toggling a single flag.


## Phase 4 role/access operations (staged backend only)

`POST /admin/portal-identity/operations` uses the dedicated bridge token and a strict body:

```json
{
  "clerk_user_id": "user_CURRENT_AUTHENTICATED_ACTOR",
  "operation_id": "00000000-0000-4000-8000-000000000001",
  "target_user_id": "00000000-0000-4000-8000-000000000002",
  "expected_version": 1,
  "change": {"action": "set_role", "role": "staff_support"}
}
```

The access form is `{"action":"set_access","active":false}` (or `true`). The two agency lifecycle commands are described below. No provider account create/delete/password, caller-selected agency or permission override is accepted. The trusted caller must derive the actor subject from the verified current login. There is no Next mutation transport or UI connected yet; the isolated preview still blocks unsafe methods. Rust verifies the provider actor and reloads current Rust authority in the mutation transaction.

Only current active managers can change permitted target roles/access; an active stored B2B owner can change access for their own stored sub-user. Self-change, terminal targets and unauthorized Super Admin changes are denied. Role grants requiring new agencies return `IDENTITY_AGENCY_PROVISIONING_REQUIRED`; incompatible membership changes return `IDENTITY_MEMBERSHIP_DEPENDENCY`. Activation behind a suspended agency/owner returns `IDENTITY_AGENCY_REACTIVATION_REQUIRED`; use the explicit `reactivate_agency` workflow below for a suspended owner/agency.

An operation UUID is bound to actor, target, expected version and typed change. Exact replay returns the original committed result after rechecking current scope. Changed payload returns `IDENTITY_IDEMPOTENCY_CONFLICT`; a new request with a stale version returns `IDENTITY_VERSION_CONFLICT`. No-op changes return `IDENTITY_NO_CHANGE`. Every change serializes the last-active-Super-Admin guard with the write.

Local authority, action limit, immutable audit, linked API access revocation and outbox insertion are atomic. Both external-user and staff-client links are included: clients/API management/credentials are disabled and machine/prebooking sessions revoked. Activation does not silently re-enable API clients. Owner suspension also suspends the agency and invalidates each sub-user's authorization version. Subs retain their own status, but effective session authority is suspended. Wallet ledger, balances, owners and immutable links are preserved.

`POST /admin/portal-identity/operations/query` accepts only `clerk_user_id` and `operation_id`, re-verifies provider/current Rust actor scope and returns the same safe result shape. Responses are no-store and include:

- `authority_mode: "staged"`, `local_committed: true`, operation ID/target/action and immutable resulting version/role/status. Agency-affecting operations also include an `agency` result snapshot (ID/code/version/status); other operations return `agency: null`.
- `state`: `pending_effects`, `needs_reconciliation` or `completed`. Local commit does not mean Clerk synchronization completed.
- `effect_count`, `effects_truncated` and at most the first 100 effect summaries (ID, kind, state, attempts, fence, allowlisted error code). Operation state covers **all** effects, including omitted summaries. Full operator queue/cursor browsing is available through the Phase 4 recovery endpoints; no lease token, raw provider response or profile/password payload is exposed.

### Provider effects and recovery boundary

Migration 0036 retains operation, effect and attempt evidence. `src/identity/effects.rs` exposes the provider interface implemented by the official Clerk adapter and synthetic fixtures. Default runtime remains read-only; the explicit disposable-test configuration enables bounded worker dispatch. There is no browser endpoint to force completion.

Each target's session revocation precedes metadata mirroring. Later unresolved effects for the same target block behind earlier work. Older pending metadata snapshots become `superseded` when the current authorization version differs; provider metadata never writes canonical roles back to Rust.

A claim commits its opaque token, incremented fence, 30-second lease, attempt row and audit **before** the provider call. Provider I/O has a 10-second deadline and holds no database transaction. The worker can confirm success, report a proven `NotSent`, or record `Unknown`. Only proven-not-sent calls retry, with a 30-second delay and five-dispatch cap. An uncertain response or expired dispatch/reconciliation lease becomes `needs_reconciliation`, never an automatic resend. Reconciliation invokes the adapter's read-only observation method and checks the expected fence; absence or `NotSent` from observation cannot prove a timed-out write did not occur. Expired/stale workers cannot finalize newer attempts; exact same-result finalization is idempotent.

Worker scans and lease recovery are bounded to 25 entries per invocation. Unknown effects block later work for the same subject while other subjects can continue. Effect/attempt immutable snapshots and allowed transition guards are enforced in PostgreSQL. Audit failure rolls back claim/result/local state writes. See [execution evidence](evidence/RUST_IDENTITY_OPERATIONS_2026-09-17.md).


## Phase 4 explicit agency provisioning and reactivation

Migration 0037 extends the existing operations endpoint with two commands. They remain backend-only and staged: no new Next mutation UI/transport is enabled, and the production provider adapter remains read-only.

```json
{"action":"provision_agency"}
```

Use this as `change` in the existing operation body with the current target user version. Only a current active Admin/Super Admin managing an eligible registered **customer** in `onboarding` or `active` state can provision. The target's existing Clerk subject is also verified before the transaction. Self-change, inappropriate target roles, suspension/tombstones, existing membership and known retained subject-level client/wallet/booking references block the operation. There is no caller-supplied agency code, agency selection, wallet ID, balance, currency, role or password.

One transaction grants active B2B role, creates a new agency and owner membership, provisions a fresh BDT wallet at zero available/held balance with no ledger posting, stores an immutable `portal_agency_wallets` binding, appends audit and queues provider effects. No API client or credit is granted. `ST-B2B` plus six digits comes from a non-recycling sequence, skipping existing registry and agency-wallet owner codes. Allocation checks at most 25 candidates; `IDENTITY_AGENCY_CODE_BUSY` can be retried with the same uncommitted operation ID. Values above 999999 return `IDENTITY_AGENCY_CODES_EXHAUSTED`. Sequence gaps after failures are intentional.

Wallet inserts are strict inserts, never the generic wallet provisioner's existing-owner upsert. A collision from a concurrent legacy writer aborts the transaction; it cannot attach a retained wallet. Known retained subject references return `IDENTITY_MATCHING_REVIEW_REQUIRED`. Historical Supabase-only agency association still requires the later matching inventory; this staged command must not be treated as a migration/import tool.

```json
{"action":"reactivate_agency","expected_agency_version":2}
```

This requires a manager, the suspended B2B owner as the target, the current user `expected_version`, the expected stored agency version, a suspended agency and its stored owner membership. The target gets a current provider check. Stale agency state returns `IDENTITY_AGENCY_VERSION_CONFLICT`; archived/non-suspended/mismatched targets return `IDENTITY_AGENCY_REACTIVATION_INELIGIBLE`. Plain `set_access:true` still cannot silently reactivate a suspended agency.

Reactivation restores the same owner and agency and invalidates member authorization versions. Active sub-users regain effective agency access; individually suspended sub-users remain suspended. Wallet identity, balances/history and frozen state are preserved. Linked API clients/credentials remain disabled and their sessions are revoked; they need their own separately authorized access workflow. Provider effects remain ordered/fenced under the existing queue rules.

Provisioning shares the 30/hour role-action bucket; reactivation shares the 40/hour access-action bucket. Alternate commands therefore cannot multiply allowances. Exact replay returns the original snapshot after current scope checks without allocating another agency/wallet or charging another action allowance. Suspension now also records its agency result snapshot, giving the caller the version needed for explicit reactivation. Current session state and immutable operation results are distinct: querying an old provisioning operation after suspension still reports its original active agency snapshot.

These commands prepare the local agency foundation for later create/invitation/application-approval workflows. They do not create a Clerk account, send invitations, record an application review decision or transfer profile documents. See [agency verification evidence](evidence/RUST_IDENTITY_AGENCIES_2026-09-17.md).


## Phase 4 durable account creation (synthetic writer verification)

Migration 0038 adds password-free prepared create intents and immutable attempts. These routes use the same staged bridge token, fresh actor lookup, 4 KiB body limit, 15-second HTTP deadline and safe error mapping. By default dispatch returns `503 IDENTITY_PROVIDER_WRITE_DISABLED`. The official create writer requires the isolated test configuration below. Existing create UI remains a later parity task; legacy preview gates remain in place.

`POST /admin/portal-identity/creates/prepare` accepts:

```json
{
  "clerk_user_id":"user_CURRENT_ACTOR",
  "operation_id":"00000000-0000-4000-8000-000000000003",
  "intent":{
    "email":"person@example.invalid",
    "first_name":"Synthetic",
    "last_name":"Account",
    "role":"b2b",
    "agency_id":null,
    "expected_agency_version":null
  }
}
```

Only current active managers may prepare/grant their allowed roles; Admin cannot grant Super Admin. Direct password creation is not a B2B-owner action (their invitation workflow is described below). Email/names are normalized and bounded. `b2b` receives its own new agency; only `b2b_sub` requires an existing active agency UUID and expected agency version. The stored owner must be active. Membership plus pending-create and pending-invitation reservations is capped at 100, including the owner; checks repeat before provider dispatch and local finalization.

The prepare transaction saves intent, actor, reserved new internal user UUID, a fingerprint of normalized **non-password** fields, email reservation and audit. It consumes the shared 20/hour per-actor create allowance; exact prepare replay does not charge again. Another unfinished intent for the same normalized email returns `IDENTITY_CREATE_ALREADY_PENDING`. The reservation prevents duplicate external create attempts; email never selects an identity, wallet or agency. Operation UUIDs cannot also be consumed by unrelated role/access commands.

`POST /admin/portal-identity/creates/dispatch` accepts only:

```json
{"clerk_user_id":"user_CURRENT_ACTOR","operation_id":"00000000-0000-4000-8000-000000000003","password":"TRANSIENT_PASSWORD_REENTRY"}
```

The password wrapper has no `Serialize`, `Debug` or `Clone`; it is bounded/validated before dispatch and only passed in memory to the provider adapter. It is never in prepared intent, hashes, retry storage or audit. This is not a claim of memory zeroization. The official Clerk adapter sends the password once and leaves provider password-policy enforcement to Clerk. A safe not-sent retry requires password re-entry; no worker can recover a password from the database.

Before an external write, dispatch commits a new token/fence, 30-second lease, attempt and audit. The provider call has a 10-second deadline and holds no database transaction. A confirmed result must include the verified server-written operation correlation, valid current provider user and matching verified contact email; email alone is insufficient proof. Wrong/missing correlation, ambiguous result, timeout or lease expiry goes to `needs_reconciliation`. Absence on a provider read never authorizes another write. Only proven `NotSent` returns to prepared with a 30-second delay; after five proven-not-sent dispatches the intent safely terminates as cancelled with `IDENTITY_PROVIDER_RETRY_LIMIT`.

The trusted internal `reconcile` method performs only a provider observation with an expected fence and durable attempt; no public force-complete/reconcile endpoint accepts browser evidence. Stale workers/readers cannot finalize a newer attempt. Uncertain and confirmed creations cannot be cancelled or dispatched again. Replaying dispatch after provider confirmation/completion returns the durable state without another create call.

`provider_confirmed` is saved **before** local finalization. The HTTP dispatch handler attempts finalization after confirmation; if it fails, query/finalize the existing operation rather than minting a new one. `POST /admin/portal-identity/creates/query`, `/finalize` and `/cancel` accept only `clerk_user_id` and `operation_id`. Current original creator or a current Super Admin with grant permission can inspect/recover; losing role/access denies further grant. Cancel is allowed only while prepared (never attempted or last attempt proved not-sent), or as an idempotent replay of cancelled state.

Finalization performs a fresh provider identity read outside the transaction, then rechecks actor grant, agency/version/capacity and immutable provider subject inside the authority transaction. It refuses existing mapped identities or retained financial/client/business references (`IDENTITY_MATCHING_REVIEW_REQUIRED`). It atomically creates the canonical user, any required agency/membership/zero-balance wallet, ordinary identity operation/effects and audit, then marks create `completed`. Customer is onboarding; authorized staff/manager/B2B/sub roles are active. Account mail intent is enqueued in this transaction; delivery and metadata/session effects run separately through the gated worker. The completed create's ID is also the ordinary operation ID for querying pending provider mirror/revocation effects.

Create responses contain safe state only: `authority_mode`, `id`, `state`, intended `role`, `local_committed`, completed internal `user_id`, `attempts`, `fence` and allowlisted `error_code`. They omit email, names, provider subject, claim token, password and raw provider responses. `completed` means local provisioning committed; optional/queued provider synchronization may still be pending in the ordinary operation. Results are snapshots, not current permission grants.

Automatic onboarding of an unregistered subject is denied with `IDENTITY_CREATE_PENDING` when its known provider ID or verified-email reservation belongs to an unfinished create. This is a denial guard, not identity matching. A subject with no verified email/correlation that races onboarding before the provider result is durably saved remains a matching-review conflict at finalization; no silent adoption/promotion occurs. The runtime supplies verified private correlation and durable webhook coordination; production writer activation remains Phase 9.

See the earlier [create/recovery slice evidence](evidence/RUST_IDENTITY_CREATES_2026-09-17.md) and the Phase 4 completion evidence for official adapter, signed inbox, mail and worker verification. Full live integration remains pending.


## Phase 4 durable invitations (synthetic adapter verification)

Migration 0039 adds invitations, fenced issue/revoke/read attempts and mail intent. These staged routes share the bridge authentication, fresh Clerk actor lookup, no-store responses, 4 KiB limit and 15-second deadline. Default runtime has no invitation writer and fails closed. The explicit disposable-test configuration enables the official adapter, with branded link resolution and verified acceptance bridges described below. Existing invitation-management UI parity remains Phase 7.

`POST /admin/portal-identity/invitations/prepare` accepts either a manager grant:

```json
{"clerk_user_id":"user_CURRENT_ACTOR","operation_id":"00000000-0000-4000-8000-000000000004","email":"person@example.invalid","grant":{"kind":"manager","role":"b2b_sub","agency_id":"00000000-0000-4000-8000-000000000005","expected_agency_version":1}}
```

or a stored B2B owner's own-agency command:

```json
{"clerk_user_id":"user_CURRENT_OWNER","operation_id":"00000000-0000-4000-8000-000000000004","email":"person@example.invalid","grant":{"kind":"own_agency"}}
```

Manager roles use the existing grant policy. Only `b2b_sub` carries an agency/version; other roles omit both or set both null. Owner input has no role/agency/version override: Rust derives `b2b_sub` and the active stored owned agency. Sub-users cannot invite. Role, active issuer/agency/owner state, expected agency version and member capacity are checked before issue and acceptance. Current allowed managers or the current active owner of the stored agency may query/revoke; an unrelated owner cannot do so. Losing original issuer authority blocks further issue/acceptance even if another manager requests dispatch; authorized managers may still revoke the outstanding invitation.

Preparation saves normalized contact/hash, intended grant, immutable issuer and reserved new user UUID, request fingerprint and audit. Operation UUIDs and unfinished email reservations cannot collide across create/invitation/ordinary operations. Reservations only deny competing onboarding/preparation; they never attach an existing identity. Both workflows reserve from the same 100-member agency limit, including the owner. Invite allowance is 20/hour per actor; manager revoke is 30/hour; owner revoke shares the existing 40/hour access-action bucket. Exact preparation/revoke replay does not consume another allowance.

`POST /admin/portal-identity/invitations/query` and `/dispatch` accept only `clerk_user_id` and `operation_id`. Dispatch issues a prepared invitation, or revokes a provider invitation whose local revocation was requested. It commits a 30-second lease, token/fence, immutable attempt and audit before the 10-second provider call. The adapter contract requires provider notification disabled and eventual preservation of the existing canonical server redirect and branded mail transport. Neither a caller redirect nor bearer invitation URL/ticket is stored or returned.

`POST /admin/portal-identity/invitations/revoke` accepts `clerk_user_id`, `operation_id` and positive `expected_version`. This is a **local durable revocation request**. It immediately sets an irreversible flag blocking acceptance and pending mail; a never-sent prepared invitation becomes `cancelled`. An already issued or uncertain invitation still needs dispatch/reconciliation to establish provider revocation. Stale versions conflict. An already accepted invitation cannot be revoked to remove a user; use the separately authorized access/deletion workflows.

States distinguish `prepared`, `issuing`, `issue_unknown`, `observing_issue`, `pending`, `revoking`, `revoke_unknown`, `observing_revoke`, `revoked`, `accepted`, and `cancelled`. `pending` means provider issuance confirmed, not email delivered. `revocation_requested:true` continues to block grants even if state is pending or unknown. Only proven-not-sent writes retry, after 30 seconds and at most five attempts. Exhausted never-sent issue is safely cancelled; exhausted revoke remains unresolved with its local deny flag. Unknown/expired claims never automatically resend. Internal read-only reconciliation requires the expected fence and exact correlation/provider ID; absence or a still-pending revoke observation does not prove completion. Late outcomes cannot overwrite newer attempts; repeating an already recorded identical result is harmless. Audit failure rolls back local claim/result/grant changes.

`POST /admin/portal-identity/invitations/accept` accepts only `clerk_user_id` and `operation_id`. The trusted Next bridge must derive that subject from the actual signed-in Clerk session. Rust verifies the provider subject, then requests independent provider evidence through the invitation adapter. Required proof: exact operation and invitation ID, accepted state, the same authenticated provider user, current ban/lock state and matching provider-verified email. Browser metadata, a matching email, a pending invitation or an uncorrelated provider user cannot grant access. The proof contract has only been exercised synthetically; official Clerk evidence/correlation and expiry semantics must be verified before wiring it.

After the read, the authority transaction rechecks pending/unrevoked state, original issuer permissions and current agency scope/capacity. A revoke committed during the read prevents the grant. Existing mapped identities or retained client/wallet/booking references return matching review. Fresh provisioning, membership/agency/zero BDT wallet as appropriate, ordinary `accept_invitation` operation, provider outbox effects and audit commit with accepted state. Customer remains onboarding. Exact acceptance replay for the same subject returns the same result; a different subject cannot reuse it. The invitation's UUID also identifies its ordinary operation for querying outstanding provider effects.

Safe response fields are `authority_mode`, `id`, `state`, `version`, intended `role`, `agency_id`, `revocation_requested`, `mail_state`, `fence`, issue/revoke attempt counts, allowlisted `error_code` and accepted internal `user_id`. Email, provider ID, lease token, ticket/URL and raw evidence are omitted. These snapshots do not grant current authority.

The original mail eligibility record remains `pending` or `blocked`; migration 0041 adds separate recipient/archive delivery jobs, fenced attempts and unknown-outcome handling. Invitation views show the recipient delivery state unless locally blocked. Provider `expired` is a terminal invitation state. Revoke/accept blocks pending mail atomically. See the earlier [invitation slice evidence](evidence/RUST_IDENTITY_INVITATIONS_2026-09-17.md) and the Phase 4 completion evidence.


## Phase 4 guarded deletion (synthetic adapter verification)

Migration 0040 and the dedicated deletion journal implement staged deletion without purging identity or financial history. Default runtime dispatch returns `503 IDENTITY_PROVIDER_WRITE_DISABLED`; official adapter dispatch requires disposable test mode. Tests use explicit fake injection and loopback HTTP fixtures. Next delete actions and live authority remain unchanged; the existing UI confirmation flow is not connected yet.

`POST /admin/portal-identity/deletions/preview` accepts:

```json
{"clerk_user_id":"user_CURRENT_ACTOR","target_user_id":"00000000-0000-4000-8000-000000000006"}
```

Only a current active Super Admin may globally delete; a current active stored B2B owner may remove their own stored sub-user. Admin cannot delete. Self-deletion is forbidden. Recovery uses the same current scope, including after the target becomes deleting/deleted; loss of the original owner's authority requires a current Super Admin to continue the existing operation.

Preview is read-only apart from the shared HTTP rate bucket. It returns staged mode, target ID/role/status, current `expected_version`, owned agency ID/code (if any), `review_token`, `blockers` and `can_prepare`. The review token is a snapshot fingerprint, not a credential or provider proof. Never treat it as a substitute for current authorization. The frontend must display this review before requesting confirmation when that UI is implemented.

| Blocker | Meaning |
| --- | --- |
| `live_members` | Owned agency still has a non-deleted sub-user, including suspended/deleting users |
| `provider_work` | Unresolved target effects, creates/invitations issued by/for the target, or pending creates/invitations for the owned agency |
| `wallet_references` | User/owned-agency wallet has funds, holds, unresolved reservations or pending requests |
| `client_references` | Retained enum for earlier snapshots; current review revokes and retains links instead of blocking on existence |
| `booking_references` | Unconsumed draft, pending/uncertain booking, uncancelled/unticketed hold or unresolved ticket/cancellation |
| `financial_history` | Target is an actor on a pending financial request or unresolved wallet operation; settled ledger history is retained |
| `last_superadmin` | Removing this active Super Admin would leave none; self-delete is already forbidden |
| `terminal_target` | Target is already deleting/deleted; query the existing operation |

Migration 0042 coordinates financial, booking and access writes with the identity mutation lock. Outstanding funds/holds, pending financial requests or reservations, unresolved bookings/drafts and provider work block deletion. Zero wallets, retained client links and settled ledger/booking history are preserved and no longer block by their existence alone. Resolve genuine obligations through their business workflows; never purge history or clear funds to bypass review. Writes are disabled by default and live provider activation remains a later gate.

`POST /admin/portal-identity/deletions/prepare` accepts:

```json
{"clerk_user_id":"user_CURRENT_ACTOR","operation_id":"00000000-0000-4000-8000-000000000007","target_user_id":"00000000-0000-4000-8000-000000000006","expected_version":1,"review_token":"COPY_THE_64_HEX_CHARACTER_TOKEN_FROM_PREVIEW"}
```

Preparation rechecks current role/scope, target version, review token and dependencies under the authority transaction. A changed review returns `IDENTITY_DELETION_REVIEW_STALE`; an ineligible target returns `IDENTITY_DELETION_BLOCKED`. Operation IDs cannot collide with other identity workflows, and one target can have only one deletion journal. Exact replay returns the same operation after current scope checks; changed payload conflicts. Global deletion is limited to 10/hour per actor; owner removal shares the existing 40/hour access bucket.

Successful preparation atomically saves the intent/audit, changes the user to `deleting`, suspends an owned active agency and revokes linked client credentials/tokens/sessions. Canonical session authority denies deleting users immediately. Known client links are retained and revoked; owned wallets are frozen. New terminal-subject dependencies are rejected by the database barrier. Deleting status cannot be reverted by ordinary access commands or database status updates. There is no cancel/reactivate shortcut for an uncertain deletion.

`POST /admin/portal-identity/deletions/dispatch`, `/query` and `/finalize` each accept only `clerk_user_id` and `operation_id`. Dispatch rechecks dependencies before the write, then commits an attempt, opaque claim token, incremented fence and 30-second lease before the provider call, which has a 10-second deadline and holds no transaction. Raw provider subject/confirmation overrides are never accepted from the caller.

States are `prepared`, `dispatching`, `needs_reconciliation`, `reconciling`, `provider_confirmed`, and `completed`. Verified results must name the exact immutable Clerk subject and operation. Only proven-not-sent calls retry, with 30-second delays and a five-attempt cap. Exhaustion retains local denial and becomes `needs_reconciliation`; it does not restore access. Timeouts/ambiguous results/expired leases never automatically resend. Internal reconciliation uses fenced, durable, read-only provider observation; a still-existing user or uncertain response cannot return to dispatchable state. The official adapter verifies the exact-subject deleted-object response. Recovery requires both a signed inbox deletion event and a fresh 404; an arbitrary 404 alone is not proof.

Provider confirmation commits separately from finalization. `/dispatch` returns that durable result; call `/finalize` after `provider_confirmed`. Finalization rechecks current recovery scope and dependencies, then atomically tombstones the same user with `deleted_at`, archives an eligible owned agency, completes the journal and appends audit. Provider or database failure cannot cause another external delete on replay. Membership, immutable identity/agency keys, contact fields and all financial/bookings/audit references are retained. Explicit PII redaction/retention policy remains separate. Deleted sub-user memberships no longer consume the shared live-member limit.

Migration 0042 serializes new financial/booking/access dependencies with deletion preparation and rejects terminal-subject writes, including writes through wallet/client links. Recovery/query/result recording also revokes discovered access without erasing ownership history. Full business actor/recipient authorization remains Phase 6; these deletion barriers do not claim that integration is complete. Query may therefore write access-revocation audit and lease recovery, while preview does not. A confirmed provider deletion with a local blocker stays `provider_confirmed` and denied for review, never falsely `completed`.

Deletion views expose staged mode, journal ID, target ID, state, `access_denied`, `local_committed` (true only for completed tombstone), attempt/fence counts, allowlisted error and current blockers. They omit the provider subject, claim token, profile and raw evidence. `access_denied:true` does not imply provider deletion is confirmed. Deletions use this dedicated journal/query; they do not fabricate a completed ordinary role/access outbox operation.

See the earlier [deletion slice evidence](evidence/RUST_IDENTITY_DELETIONS_2026-09-17.md) and the superseding [Phase 4 completion evidence](evidence/RUST_IDENTITY_PHASE4_COMPLETION_2026-09-17.md). No live deletion or Supabase identity cutover has occurred.


## Phase 4 runtime, mail and recovery

Migrations 0041–0042 are additive. They add the durable event/mail journals, permanent provider-deletion evidence, provider invitation expiry and terminal-subject business barriers. They do not backfill or send old mail. Existing data is retained. The runtime is disabled by default; no `.env` or `.env.local` was changed.

Rust configuration, in addition to the staged bridge:

- `PORTAL_IDENTITY_PROVIDER_WRITES=disabled` is the default. `test` requires a `sk_test_` secret and a loopback database ending `_identity_test`; `sk_live_` and remote/non-disposable databases cannot enable writers. `PORTAL_IDENTITY_APP_ORIGIN` is a validated canonical origin for invitation redirects. There is no live writer mode.
- `PORTAL_IDENTITY_EVENT_TOKEN`: independent `stie_` + 43 base64url characters. Next uses the matching `SHAPON_IDENTITY_EVENT_TOKEN` and its separate `SHAPON_IDENTITY_WEBHOOK_SECRET` (`whsec_`). Subscribe only the isolated provider's user.created/updated/deleted events to Next `/api/identity/events`. Do not repoint existing live webhooks as part of this phase.
- `PORTAL_IDENTITY_MAIL_TOKEN`: independent `stim_` + 43 base64url characters matching Next `SHAPON_IDENTITY_MAIL_TOKEN`. `PORTAL_IDENTITY_MAIL_ORIGIN` must be loopback HTTP. This requires test writer mode. Next additionally requires `SHAPON_IDENTITY_MAIL_DELIVERY=test`, isolated `rust-preview`, and a disposable loopback SMTP fixture (`SMTP_HOST=127.0.0.1` or `localhost`). Never use a relay to real recipients in verification. Existing branded templates and SMTP single-attempt transport are reused; no live email configuration was changed.

The worker starts with the staged runtime and stops on service shutdown. Each tick processes one inbox claim, one provider-effect delivery, a rotating page of at most three lifecycle intents, one unknown-effect observation and one mail delivery. Claim/finish transactions hold no network I/O. PostgreSQL fences prevent concurrent workers or stale completions from claiming success. A shutdown during an outbound call leaves durable evidence for lease recovery. Prepared password creates require explicit resubmission because passwords are never saved; the worker only observes/finalizes those creates. Other prepared work can dispatch only through the explicitly enabled test adapters. Original issuers are rechecked for current provider/local authority. A lost issuer is not replaced with a randomly selected administrator.

The signed Next inbox verifies raw bytes with Clerk's SDK before reading the event timestamp, which the SDK omits from its returned resource. It forwards only event ID, SHA-256, subject, type and millisecond timestamp over the dedicated event capability. Same event/same content is acknowledged idempotently; collisions conflict. A 2xx means PostgreSQL accepted the envelope. Failed persistence returns an error for provider retry. Signed deletion immediately suspends linked local access and permanently bars subject reuse, including out-of-order or previously unmapped subjects. Update/create events only refresh current verified contact fields: they never grant roles, activate accounts or undo deletion. Bounded retries become visible dead letters after five attempts.

Mail jobs are created atomically with invitation confirmation, local account provisioning or onboarding. Each primary recipient and archive copy has its own immutable job and attempt history. Rust claims for 90 seconds; the Next receiver must acquire the exact single-use `(id, token, fence)` before its one SMTP attempt (40-second deadline). Duplicate HTTP delivery cannot send again. Definitive rejection backs off 30 seconds with five attempts maximum; ambiguous acceptance, timeout or expired lease becomes `unknown` and is never automatically resent. Revoked/expired invitations and ineligible users are blocked before transport; an already started email cannot be recalled, but cannot override invitation acceptance guards. Invitation links resolve through `/identity-invite/{provider_id}` after both local eligibility and a bounded provider pending-invitation read. `/api/identity/invitations/accept` derives the authenticated subject and backend invitation correlation; Rust independently verifies exact acceptance proof.

Additional Rust POST endpoints:

| Route | Capability | Body / result |
| --- | --- | --- |
| `/admin/portal-identity/events` | `stie_` | Strict sanitized event envelope; durable `{accepted,duplicate}` |
| `/admin/portal-identity/mail/start` | `stim_` | Exact `id`, `token`, `fence`; one transport start only |
| `/admin/portal-identity/invitations/link` | `stib_` | Exact `provider_id`; returns availability only, no contact/role/token |
| `/admin/portal-identity/recovery/queue` | `stib_` + current active Super Admin | `clerk_user_id`, `limit` 1–50, optional `after:{at,kind,id}`; stable cursor pages, state/fence/attempt/code only |
| `/admin/portal-identity/recovery/command` | `stib_` + current active Super Admin | `clerk_user_id`, `kind`, `id`, `fence`, `action`; provider observation, verified finalization, invitation refresh or dead-letter event read retry |

Next `/identity-recovery` provides the queue and actions. Its `/api/identity/recovery` bridge verifies a fresh Clerk session, derives the subject, rejects caller subject injection, validates response shape and requires same-origin POST. Unsafe legacy routes and Server Actions remain blocked. Forbidden/conflicting authenticated bridge actions write separate immutable denial audit without passwords, request bodies or arbitrary metadata.

Recovery rules:

1. Refresh the saved result before acting. `needs_reconciliation`, `issue_unknown` and `revoke_unknown` use provider reads; they never repeat a potentially successful write. Stale fences require refresh.
2. A provider-confirmed create/delete may finish only after the existing fresh provider/scope/dependency checks. A revoked or expired invitation never grants access. Existing provider accounts are not adopted by email.
3. Unknown mail remains visible for support to inspect provider/SMTP evidence; there is no public force-sent or resend command. An unresolved provider outcome may legitimately remain queued. Preserve it for operator review instead of changing journal state with SQL.
4. Resolve funds, holds, pending booking/draft and member dependencies through their original workflows before deletion. Retain all historical ledger, booking, agency and membership records.
5. External deletion of the last provider Super Admin still denies access. This does not authorize automatic promotion or bootstrap reset. Preserve the audit/database and perform a separately reviewed recovery/restore; never erase the bootstrap marker or infer privilege from metadata.

The fixture browser screenshot and exact executed checks are linked in the Phase 4 evidence. Full account/profile/approval and business UI parity remain later phases, and production provider/SMTP delivery was not exercised.

## Phase 5 staged profile/staff text records

Migration 0043 adds `portal_identity_profiles` and an immutable mutation journal. It does not import legacy profiles, touch account authority, create wallets or enable the dashboard's legacy actions. The new `lib/identity/profiles.ts` Next adapter is server-only and requires isolated `rust-preview`; it resolves a fresh Clerk session internally. Browser routes/UI integration remain later work. See [profile evidence](evidence/RUST_IDENTITY_PROFILES_2026-09-17.md).

All three endpoints require the dedicated `stib_` bridge credential, current provider verification, bootstrap and current Rust actor authorization. Existing 4096-byte JSON and global 600/minute bridge limits apply. Use bounded partial patches; a full form containing many maximum-length fields may exceed 4096 bytes. Unknown envelope/field keys are rejected. Do not send login credentials, caller roles or owner IDs.

| POST suffix under `/admin/portal-identity/` | Required body fields | Result |
| --- | --- | --- |
| `profiles/query` | `clerk_user_id`, `target_user_id` UUID, `kind` (`profile` or `staff`) | Current permitted fields, target identity version, record version and `exists` |
| `profiles/edit` | Query fields plus `operation_id` UUID, `expected_identity_version`, `expected_version`, `change` | Atomic versioned patch/removal and immutable journal/audit |
| `profiles/branding` | `clerk_user_id`, `agency_id` UUID | Canonical owner UUID, immutable agency code, owner profile version and six business contact fields |

Edit examples for `change`:

```json
{"action":"patch","fields":{"givenName":"Synthetic","passportNo":null}}
```

```json
{"action":"remove_staff"}
```

A missing key is preserved. `null` or a trimmed empty string stores an explicit empty string; old applications are never consulted on reads. Each field is at most 500 Unicode scalar values; control characters are rejected. Nonempty email, gender, date and website fields are validated. File/asset fields are not accepted here. `remove_staff` requires `kind=staff` and retains a tombstone/version; deleting/truncating profile rows or modifying journal history is rejected by database triggers.

A never-created record has `exists=false`, `version=0`, `fields={}`. A removed staff record has `exists=false`, an incremented nonzero version and empty fields. Store/audit failures return an error, never a missing record. New writes require both current versions. A successful edit returns `committed_version`; an identical operation replay reauthorizes current scope and returns the **current** view with `replayed=true` and the original `committed_version`. Changed content under that operation ID conflicts. Journal hashes are normalized payload digests; audit contains identifiers and versions, not profile values.

Permissions preserve the current source:

- Active Super Admin/Admin may correct profiles within their existing target-role grants, including their own profile. Admin cannot read/edit a Super Admin profile. They may correct nonterminal suspended targets.
- Other active users may read their own profiles. As requested on 2026-09-21, the Rust-backed Company profile lets active B2B owners and sub users fill blank personal/company/bank text fields in their own record. Once a field has a nonempty stored value, they cannot replace or clear it; only an authorized Admin/Super Admin can correct it. Values populated by application approval, import or an administrator are also locked. An unchanged saved value may accompany a patch of another blank field. A replacement returns HTTP 403 `IDENTITY_PROFILE_FIELD_LOCKED`; a stale version returns HTTP 409. The current-value check and write share the same transaction; a denied mixed patch saves nothing. Missing and explicitly empty fields can be filled. Document/logo mutation permissions follow the owner renewal policy below; staff records retain their existing rules. Legacy actions remain disabled under Rust authority.
- Staff roles have the ten shared personal/contact fields. Customers additionally have six bank fields. Manager edits for B2B/B2B-sub profiles include seven company fields as well. Fields retained from a previous role are hidden unless allowed by the current role.
- Six-field staff records belong only to B2B sub users. The sub user or the actual owner of their active agency may read/edit/remove them. Global managers and unrelated agencies do not gain staff-record access implicitly. Suspended/deleted actors and suspended/archived agencies cannot use membership permissions. Terminal/provider-deleted targets are denied.
- Branding resolves from the stored agency owner, never from a sub user's company text. It exposes only agency name/address/email/mobile, website and Facebook page. Passport, license, personal contact and bank fields are excluded. Logo/assets remain pending.

Successful profile/staff saves share a 20-per-five-minute actor allowance for nonmanagers; manager edits use 40/hour and manager profile reads 60/hour. SQL writes, rate counters, version increments, journal and safe audit commit together. Replays require current authorization and do not consume another edit allowance. Read and replay audit failures also fail closed. Onboarding actors cannot yet use these endpoints; the separate B2B application workflow remains pending.

## Phase 5 completed: applications, documents and sub-user names

The earlier profile section describes the first slice; [Phase 5 completion evidence](evidence/RUST_IDENTITY_PHASE5_COMPLETION_2026-09-17.md) supersedes its pending-work notes. Applications, private document references/transport, canonical logo, owner rename, staged adapters and browser workspace are now implemented. Existing production form integration remains Phase 7; business paths remain gated.

All routes below are POST under `/admin/portal-identity/`, require `stib_`, a freshly provider-verified actor and current Rust authority. Envelopes reject unknown fields. UUIDs identify Rust identities/assets; Clerk subject is injected by the server adapter, never accepted from browser input. JSON limits are 4096 bytes except `applications/submit` (32768). Requests do not contain file bytes.

| Suffix | Contract |
| --- | --- |
| `profiles/directory` | Actor-scoped `after` UUID/`limit` (1–50) cursor directory; managers see granted targets, B2B owner sees own members, others self. No provider roster or metadata grants. |
| `applications/query` | `target_user_id`; own or permitted manager read; returns current application/profile/identity versions, fields, attached asset IDs, reviewer/note and epoch-ms timestamps. Missing application is version 0 with null status/fields; outage is an error. |
| `applications/queue` | Manager-only pending list with `after` UUID/`limit` (1–50), user/version/submission timestamp and next cursor. |
| `applications/submit` | `operation_id`, `expected_version`, `expected_identity_version`, eight required camelCase `fields`, `documents` (0–5 unique ready owned application asset UUIDs). Customer self only, onboarding or active; absent/rejected only. Normalized identical replay does not resubmit. |
| `applications/review` | `operation_id`, `target_user_id`, `expected_version`, `expected_identity_version`, `expected_profile_version`, `decision` (`accept`/`reject`), `note` (≤500 characters). Active manager, current customer pending application, fresh target provider verification. Approval and all local grant/profile/wallet/outbox/audit work are atomic. |
| `documents/query` | `target_user_id`, `purpose` (`profile`/`application`), `slot`, `asset_id` (null for profile; a currently attached ID for application). Reauthorizes exact reference and writes read audit before returning a signing descriptor to trusted Next. Logo resolves through canonical agency owner. |
| `documents/policy` | Profile-only query envelope (`target_user_id`, `purpose: "profile"`, `slot`, `asset_id: null`). Returns `target_user_id`, `slot`, `can_upload`, `can_remove`, `cycle` and nullable `next_upload_at` (Unix milliseconds). Cycles: `anytime`, `july_year`, `two_years_from_upload`, `admin_only`. Current authority and audit required. |
| `documents/prepare` | `operation_id`, target/purpose/slot, expected slot/application and identity versions, inspected `format`, `byte_size`, SHA-256 `content_hash`. Returns reserved `shapon/identity/<UUID>` handle and durable prepared intent. No supplied URL or public ID. |
| `documents/start` | `asset_id`; original uploader, current authority/versions, prepared state only. Commits uploading state before provider I/O. A second start conflicts. |
| `documents/finish` | Trusted server only: `asset_id`, `outcome` (`ready`/`unknown`/`abandoned`), exact reserved public ID/format/size/hash. Ready requires fresh authority and parent versions. Only then is a profile slot linked. Browser routes cannot call this attestation action. |
| `documents/intent` | Original uploader's `asset_id`; observes durable upload state without signing or granting access to detached content. |
| `documents/uploads` | Original uploader's target-scoped `after` UUID/`limit` (1–50) list; supports reload/recovery and optional reuse of owned ready application attachments. |
| `documents/remove` | `operation_id`, target/slot, expected slot/identity versions. Manager company correction or active owner self-logo removal; increments a null-reference tombstone. Replay returns the current slot and cannot erase a later replacement. Storage history is retained. |
| `sub-users/rename` | `operation_id`, target UUID, `expected_version`, trimmed `first_name`/`last_name` (≤60 characters each, at least one nonempty). Active actual agency owner only. Stores canonical display name plus immutable `mirror_name` effect atomically; no local role or membership change. |

Application fields: `agencyName`, `businessMobile`, `businessEmail`, `businessAddress`, `fullName`, `businessType` (`proprietor`/`partner`), `personalMobile`, `personalAddress`. Every field is required and limited to 500 characters. Rejected resubmission clears the old current reviewer/note but retains immutable history. Approval copies only profile keys that have never been set; empty strings remain deliberate removals. Generic supporting attachments are not misclassified into named company-document slots.

Profile document slots: `tradeLicense`, `tinCertificate`, `travelAgencyLicense`, `nidCard`, `logo`; application assets use `attachment`. Non-logo documents allow PDF/PNG/JPEG/WebP ≤5 MB. Logo allows SVG/PNG/JPEG/WebP ≤512 KB using existing byte validation; SVG logo display uses PNG delivery. Assets store only private provider handles, hash/format/size, uploader/target/purpose and timestamps. Invalid receipt, stale publication and unavailable audit fail closed.

Active B2B owners may upload their own logo anytime, their trade license once per July–June year, and their CAAB license once every two calendar years measured from the last successful upload date/time. Calendar calculations use Bangladesh time; July resets at midnight and a leap-day CAAB anniversary clamps to 28 February. `travelAgencyLicense` remains the stored slot key and is labeled CAAB License. The first upload counts. Rust checks the retained successful asset history at prepare/start/ready-finish; failed/unconfirmed uploads do not consume the allowance. Early renewal returns HTTP 409 `IDENTITY_DOCUMENT_UPDATE_NOT_DUE`. Owner license removal is denied; even admin removal retains the renewal history. Admin replacements count as successful uploads, while Admin/Super Admin can override the owner's wait. Sub users cannot change shared company documents/logo, and TIN/NID remain admin-managed. The frontend uses `documents/policy` to show permissions and the next date; server checks remain authoritative. No new migration is required.

Next `/api/identity/phase5` exposes an allowlisted `{action,data}` browser envelope with same-origin/session checks. `/api/identity/documents` accepts one bounded multipart file plus strict input metadata, inspects bytes, reserves/starts via Rust, uploads authenticated with no overwrite, validates receipt and finalizes. Recovery reads Cloudinary; it never blindly re-uploads. Signing consumes Rust-authorized current references only. `/identity-phase5` provides isolated profile/staff/application/document/name journeys. The asset adapter is disabled unless the disposable-cloud test gate is explicitly configured; production cannot select `rust-preview`.

Activation email uses the existing branded B2B template and recipient/archive durable mail intents. Both claim and transport-start check current B2B/agency access. The new name effect uses `PATCH /users/{id}` without metadata changes. Immediate matching responses can confirm; ambiguous writes remain in reconciliation because equal names alone cannot prove a delayed request completed. Existing recovery UI exposes that state without a blind retry control.

## Phase 6–8 staged business/UI integration (2026-09-17)

This section supersedes earlier “next phase”/integration-pending notes for the surfaces listed here. See the [runbook](RUST_IDENTITY_RUNBOOK.md), [dependency inventory](RUST_IDENTITY_DEPENDENCY_INVENTORY.md) and [verification evidence](evidence/RUST_IDENTITY_PHASE6_8_2026-09-17.md). Default/live modes are unchanged; Phase 9 activation is pending.

| POST endpoint under `/admin/portal-identity/` | Credential / contract |
| --- | --- |
| `readiness` | `stib_`; non-PII staged schema/bootstrap, pending/uncertain work and provider/mail/inbox availability. `live_activation_available` remains false |
| `roster` | `stib_`; verified `clerk_user_id`, bounded search/role/status/date/sort/page/limit. Manager directory or actual B2B owner's sub-user scope. Returns canonical versions, agency state, summary and bounded invitation preview |
| `business/execute` | `stib_`; verified `clerk_user_id`, closed `path`/`method`, object `body`. Rust derives/revalidates actor, agency owner, target and managed-client context. Not a browser endpoint or generic admin proxy |
| `business/directory` | `stib_`; verified actor, `kind` (`receivers`, `assignees`, `recipients`), `query`, `after` UUID/null, `limit` 1–50. Explicit role/scope checks and next cursor |
| `business/lookup` | `stib_`; verified actor and exact subject; manager/self/agency/receiver scope. Unprivileged receiver lookups redact email |
| `business/notification-directory` | `stib_`; exact `event_id` and live `claim_token`. Requester/reviewers/company contact only for an actively claimed wallet preparation; cannot enumerate arbitrary recipients |
| `wallet/notifications` | Independent `stim_` service capability, not a browser subject or generic administrator. Existing wallet worker commands, leases, immutable plans, fencing, acknowledgement replay and unknown-delivery rules |

The business allowlist covers existing Rust portal wallet/nonissuance/settings, passengers, native holds, prebooking sessions, supplier-name disclosure, API-client/tier management and authorized OpenAPI views. Paths/methods/query keys outside it are denied. Browser-supplied actor/role/owner mismatches are denied; display snapshots and cash receivers are regenerated from canonical data. Client provisioning binds only a verified agency's existing canonical zero-balance wallet and never imports/relinks financial ownership or enables external API permissions.

The wrapper creates an unforgeable request-local context. Domain mutation transactions recheck current actor/target authorization and relevant client versions under the identity authority lock. The lock is released before provider/supplier I/O. Already dispatched operations retain their existing settlement/reconciliation paths. API-client-management mutations retain a shared 30/hour actor limit. Business envelope limit is 32 KiB/120 seconds; notification worker body limit is 512 KiB/15 seconds; other identity limits remain unchanged. Never use an HTTP timeout as evidence of no provider/supplier write.

Migration 0045 introduces separate five-minute `sti_` search sessions bound to canonical user/authorization version/client. They authorize only Search/FareRules/Reprice/pricing and eligible price acceptance, never Book/Issue. Authorization changes delete affected sessions. Price acceptance also rechecks the canonical actor inside its mutation transaction. Legacy machine credentials/scopes remain separate.

Native booking submission remains Super Admin on behalf / B2B owner. Manager/Support/Accounts booking reads are global; sub-user reads use their canonical agency owner. Extra reader roles are accepted only inside the dedicated canonical context. Supplier cost/profit remains Super Admin only. Legacy generic-bridge callers cannot acquire these extra roles by changing JSON.

Next retains existing dashboard/flight URLs and dashboard shell, with ported navigation, typed access/setup errors, profile/security widgets, account forms, pending operation state, previews, search/filters/sorts and current versions. `POST /api/identity/accounts` is a strict same-origin action envelope. Recent-sign-in/avatar hydration reads only Clerk authentication presentation fields for canonical IDs and never uses role metadata. More than 1000 matched records for recent-sign-in sorting gives an explicit bound error. Other roster sorting remains server-paginated.

The existing signed Clerk auth-email relay is available with test-only local SMTP; application welcome is Rust-owned. The wallet cron route remains bearer-authenticated and uses `stim_`, while interactive business calls use `stib_`. All real SMTP/SMS/storage writes stayed disabled during verification. Generic legacy identity table/RPC/action paths fail closed in preview, including when Supabase is absent.

## Phase 9 preparation — read-only preflight and maintenance

`POST /admin/portal-identity/preflight` requires the temporary `stio_` operator capability and strict `{ "clerk_user_id": "user_SELECTED_OPERATOR" }` JSON. Bridge, event, mail and machine credentials cannot call it. It returns schema/bootstrap checks, the selected local active-Super-Admin check, 14 unresolved queue summaries, eight mapping issue counts, 11 bounded mapping table fingerprints and explicit external activation blockers. No provider lookup or mutation occurs; `operator_provider_verified`, `activation_ready` are always false. A local database identity check must not be represented as proof of a current Clerk sign-in.

`readiness` retains existing fields and adds `backlog`, `unresolved_work_items`, `recovery_clear`, `maintenance_enabled`, and `worker_paused`. `uncertain_operations` now counts uncertain/review-required rows across every reported queue, including mail/assets/events and business reservations; counts are not deduplicated across a logical operation. Ticket backlog uses the existing `flight_ticket_outcomes` view so confirmed reconciliation/nonissuance is respected. Manually resolved bookings are terminal. `recovery_clear` covers bootstrap and queues only; preflight separately reviews mappings and selected operator.

Both inspection endpoints now use repeatable read/read-only transactions, skip boundary rate-limit/audit writes and support SELECT-only DB permissions. Schema mismatch, unavailable storage, timeout or the 100,000-row-per-mapping-table review bound fails the request. Existing body/time limits remain. No report offers an activation/force option.

Rust `PORTAL_IDENTITY_MAINTENANCE=true` rejects all requests except `GET /health/live` and these two authenticated POST inspection endpoints. It pauses the identity worker and prevents startup of the cleanup worker; business readiness is 503. Next `SHAPON_IDENTITY_AUTHORITY=maintenance` rejects matched portal/API/actions before legacy or canonical work. Both return 503/no-store/Retry-After for blocked requests. Configuration defaults remain unchanged and all replicas must be drained/restarted together; these controls do not prove external/old workers are stopped and do not implement a durable production authority marker.

See the [runbook](RUST_IDENTITY_RUNBOOK.md) and [verification](evidence/RUST_IDENTITY_PHASE9_PREFLIGHT_2026-09-17.md) for the private no-overwrite CLI artifact and drift check. Phase 9 actual target configuration and activation remain outstanding.

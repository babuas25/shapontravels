# Deployment and maintenance runbook

Current operating instructions, updated 2026-09-21. Read this before deploying,
changing Vercel environment variables or changing identity configuration. This
supersedes the historical single-server `main` / `VPS_*` deployment instructions.
Recorded revisions below are evidence of a completed release, not values to copy
into a future release. Recheck the actual target before making changes.

## Repositories and environments

| Component | Development | Production |
| --- | --- | --- |
| Rust repo `babuas25/shapontravels` | branch `development` | branch `production` |
| Frontend repo `babuas25/shapontravels-frontend` | branch `development` | branch `main` |
| VPS | `160.25.226.72` | `160.25.226.236` |
| Rust API | `https://sendbox.shapontravels.com` | `https://api.shapontravels.com` |
| Runtime `APP_ENV` | `uat` | `production` |
| Vercel environment | Preview, scoped to Git branch `development` | Production |

Development frontend: <https://shapontravels-frontend-git-development-shapontravels.vercel.app/>

Production frontend: <https://shapontravels-frontend.vercel.app/>

Use the stable branch URL for development checks. An old immutable Vercel
deployment can retain an obsolete identity pin and intentionally fail after a
coordinated rollout. Keep Vercel deployment protection enabled.

The two environments have separate databases, identity bridge secrets and rollout
markers. Never point Preview at production to fix a development failure. New work
starts on `development`; production promotion needs the user's release approval.
Inspect branch, remote and uncommitted changes in both repositories before work.
A general backend push request does not include the frontend. The authorized
frontend remote is the new private repository above, never `babuas25/shopontravels`.

## Ordinary development release

1. Review the diff, migrations and affected checks. Preserve unrelated local work
   and keep `.env*`, `.local`, credentials and private evidence out of commits.
2. Decide whether the change preserves the current identity contract and pin.
   Initial activation or a changed authority pin requires the coordinated
   procedure below; ordinary CI is not a substitute for that procedure.
3. Push the authorized backend changes to `development`. Wait for the matching
   GitHub Actions run's checks, build **and Deploy development job** to succeed.
   A successful build alone does not establish that the VPS is running it.
4. Verify the deployed SHA, both health endpoints and authenticated identity
   readiness, including the durable marker and runtime pin. Resolve backend
   readiness before publishing the dependent frontend.
5. Test the frontend changes, then push to its `development` branch when requested.
   Its Vercel variables must be scoped to Preview + Git branch `development`.
   Other Preview branches need their own reviewed configuration.
6. Wait for the matching Vercel deployment to be READY. Verify the stable branch
   homepage in the browser and the existing user's login, dashboard and role.
   Do not create real bookings, tickets, deposits or notification deliveries as
   an incidental deployment check.

The frontend Preview build checks authenticated backend readiness before and
after `next build`. It rejects maintenance, an inactive authority, rejected bridge
or mismatched pin. This protects publication at check time; it cannot guarantee
future backend availability. Runtime authorization remains enabled. The gate is
currently on frontend `development`; Production builds do not run this gate.

Frontend regression checks for the recent deployment failures:

```sh
node --test scripts/test-preview-readiness.mjs
node scripts/verify-optional-public-images.mjs
npm run typecheck
```

Run these in the frontend repository. Run relevant application checks as needed;
do not treat these three checks as a complete test suite for every change.

## Production promotion and coordinated identity changes

For an ordinary compatible release, promote reviewed backend changes to
`production` and frontend changes to `main` only with release authorization.
Verify production-specific configuration and backend readiness before publishing
the dependent frontend. Do not copy Preview environment variables into Production.

For initial activation or a changed identity pin, follow the complete
[Rust identity runbook](RUST_IDENTITY_RUNBOOK.md):

1. Select the exact environment, current Clerk operator, database and release
   pair. Take a target backup and verify restoration into a new disposable DB.
2. Coordinate maintenance, stop/drain all relevant writers and workers, and
   review retained mappings, pending work and unknown provider outcomes.
3. Obtain fresh private preflight evidence. Review a new rollout plan, apply the
   selected activate/pause/resume transition and retain its result. Never replay
   an old `.local` plan or blindly retry an uncertain apply.
4. Coordinate Rust runtime configuration, the durable marker and all Next pin
   fields. Publish the selected frontend only when its backend authority is ready.
5. Remove temporary operator capabilities, leave maintenance, and verify the
   complete application and required workers. Record the final revision and SHAs.

The normal activation helper expects `/health/ready` to return 200. Maintenance
intentionally returns 503, so review the maintenance installation/restart sequence
separately. Do not bypass the normal helper's health checks. Identity release
fingerprints are not interchangeable with Git commit SHAs.

## Server administration and verification

See [server configuration and Actions variables](PRODUCTION_SERVER.md) for the
exact deployment credentials, branch switches, service files and helper checksums.
The deployment helper backs up, stops the API, migrates using a separate role,
reapplies application grants, activates the verified binary and checks health.
It can cause restart downtime and does not roll back database changes.

On the explicitly selected VPS, these checks are read-only:

```sh
cat /opt/shapontravels/deployed-sha
systemctl is-active shapontravels.service
curl --fail --silent --show-error http://127.0.0.1:8080/health/live
curl --fail --silent --show-error http://127.0.0.1:8080/health/ready
systemctl is-active shapontravels-notifications.service
systemctl list-timers --all shapontravels-backup.timer
```

Also check `/health/live` and `/health/ready` through the selected public HTTPS API.
Health does not establish canonical identity readiness or supplier entitlement.
Use the authenticated identity review tooling with secrets supplied privately;
never paste bridge/operator tokens or full environment files into logs or chat.

Production requires an active business notification worker. It has `PartOf` the
API service, so check it explicitly after API stop/start; do not assume a successful
API deployment restarted it. If stopped, validate the configured notification owner
and activation state using [the notification guide](BUSINESS_NOTIFICATIONS.md)
before restarting `shapontravels-notifications.service`. Development intentionally
does not run real business delivery. Never enable it as a generic health fix.

Runtime secrets live in `/etc/shapontravels/.env` (root:shapontravels, 640); the
service reads `.env`, not the local development `.env.local`. The migrator URL is
in `/etc/shapontravels/migration-database-url` (root-only, 600). Supplier credentials
and business SMTP/SMS settings belong in Rust configuration, not Vercel.

Backups live in `/var/backups/shapontravels` with seven-day local retention.
Production has a daily backup timer. Development release backups work, but a daily
timer was not installed at the last verification. Independent off-server backup
storage and failure notifications remain outstanding. A dump listing is not a
restore test. Never restore a rehearsal over the active database.

## Troubleshooting recurring frontend failures

| Symptom | Investigation and correction |
| --- | --- |
| `IDENTITY_AUTHORITY_UNAVAILABLE` | Inspect the matching deployment's diagnostics, API reachability, bridge authentication, maintenance and all runtime/durable/Next pin fields. This is a generic failure, not proof of one particular cause. |
| Same error on localhost:3000 | Check the local Rust listener on 18081 and its startup log. VPS migrations do not migrate the retained local database. Follow [local recovery instructions](LOCAL_PORTAL.md#after-updating-backend-code); back up and apply reviewed local migrations before restarting when required. |
| `PREVIEW_BACKEND_UNREACHABLE` | Check the development API, DNS/TLS and service availability. |
| `PREVIEW_BRIDGE_REJECTED` | Check the selected environment's bridge configuration; do not substitute the production secret. |
| `PREVIEW_BACKEND_IN_MAINTENANCE` / `PREVIEW_BACKEND_NOT_ACTIVE` | Finish the reviewed backend activation before publishing Preview. |
| `PREVIEW_ROLLOUT_MISMATCH` | Compare every pin field with current backend readiness and durable state; coordinate the release rather than editing one side blindly. |
| `IDENTITY_ASSET_DISABLED` on the homepage | Development disables assets. Optional public marketing images now fall back to no image; upload/delete/private asset restrictions stay enforced. Verify the frontend contains that fix. |
| API healthy but homepage broken | Inspect Vercel runtime logs and load the actual homepage; API health does not exercise server-rendered page dependencies. |

The 2026-09-20 Preview incident first exposed publication before development
canonical activation, then an optional marketing-image dependency that threw when
assets were disabled. Both needed separate fixes. Disabling authorization, copying
production credentials or enabling external writes is not an appropriate repair.

After canonical writes, recover forward. Do not switch back to stale permission
stores, drop identity tables, restore an old DB over financial/provider writes or
retry unknown delivery/provider outcomes blindly. An older frontend must still
match the active authority pin; reverting its commit alone may not restore service.

## Last verified release and maintenance record

Development updated 2026-09-21; production last released 2026-09-21.
Recheck before the next release:

| Item | Development | Production |
| --- | --- | --- |
| Rust release | `eee8224` | `eee8224` |
| Applied migration | `0063` | `0063` |
| Canonical rollout revision | `3` | `5` |
| Frontend release | `5178694` | `5178694` |
| Real business notification worker | Disabled | Active |

Temporary operator capabilities were removed and maintenance was off on both
targets. These historical numbers are not a reusable rollout manifest. Private
release evidence remains under ignored `.local/development-setup` and
`.local/notification-rollout`; do not commit their secrets or replay their helpers.
The separate localhost:3001 frontend is a production-connected profile, not the
development Preview merely because the checkout is on `development`.

After each release record the date, target, backend/frontend SHAs, Actions run,
Vercel deployment, applied migration, identity revision, health/browser/worker
results and outstanding follow-ups. Keep public instructions current and store
private backup/preflight/rollout artifacts separately with restricted permissions.


### Development login recovery release — 2026-09-20 17:54 UTC

- Frontend `development`: `12dd83967392fcd19be893270aa6e1656dfb24b3`, pushed
  to `babuas25/shapontravels-frontend`. Adds bounded read retries, safe correlated
  verification diagnostics and a fresh-verification Try again form.
- Vercel Preview `dpl_8H3Wh4mK4rgGXA4Yw5nSLQvaUcKu` is READY:
  <https://shapontravels-frontend-jwe59yj28-shapontravels.vercel.app/>.
  The stable development alias points to this deployment. Both pre-build and
  post-build authenticated backend readiness gates passed; compilation succeeded.
- Backend remains `373e5f6ab4242bc1c860d66ecfd60627a0aa9657`, migration `0062`,
  identity revision `3`. Runtime and durable pins matched; maintenance was off.
  Internal and public live/ready health checks passed. The real notification
  worker remains intentionally inactive in development. No backend Actions run,
  service deployment, database migration or authority-pin change was required.
- Validation passed: typecheck, login resilience, identity bridge/authority/runtime,
  Preview readiness (7 tests), optional public images and diff whitespace checks.
  Browser Google sign-in with the existing Super Admin succeeded on the stable
  development alias; the account showed Super Admin and the homepage correctly
  routed the signed-in account to its dashboard on the new deployment.
- Production was not changed. The original intermittent failure's exact cause
  remains unknown; future verification failures now carry a support reference.
  Private deployment/build evidence: `.local/development-setup/login-recovery-*`.


### Development rendering-signal correction — 2026-09-20 18:00 UTC

- Frontend `development`: `f127621199fc5c820c25c2e98ad27648a1912762`.
  Next.js rendering and redirect signals now pass through identity error handlers
  unchanged, including wrapped causes, without being logged as Clerk failures.
  Real verification errors still deny access and use the existing bounded retries.
- Vercel deployment `dpl_7aQhanG6EqR41hibnv8HdQfFxVDn` is READY:
  <https://shapontravels-frontend-nvow78qgv-shapontravels.vercel.app/>.
  Stable development alias confirmed. Complete build events show zero
  `[identity-verification]` messages, all 43 static-page discovery steps complete,
  successful compilation, and both authenticated readiness checks passing.
- Added regression cases reproduced the old swallowing behavior before the fix;
  rendering/redirect signals from Clerk and Rust reads now propagate with no
  logging or retry. Typecheck, identity bridge/authority/resilience, Preview gate
  (7 tests), optional public images and whitespace checks passed.
- Browser navigation through the stable homepage reached the authenticated
  Super Admin dashboard on the new deployment. Public API live/ready checks passed;
  canonical authority remained ready at revision 3 with matching runtime/durable
  pins and maintenance off. Backend remains `373e5f6`, migration `0062`; no backend
  push/Actions run, service restart, migration or notification-worker change.
  Production remains unchanged. Private evidence: `.local/development-setup/render-signal-*`.

### Development Triplover UAT configuration — 2026-09-21 (Asia/Dhaka)

- Installed the user-provided Triplover UAT user/search endpoints and account
  credentials in `/etc/shapontravels/.env` on development `160.25.226.72` only.
  Preserved root:shapontravels ownership and mode 0640; retained a root-only
  pre-change environment backup. No credential values are recorded in Git.
  Restarted `shapontravels.service`; internal and public live/ready checks passed.
- Authenticated identity readiness remained canonical-ready at revision 3, with
  matching runtime/durable pins and maintenance disabled. The notification worker
  remains inactive. Backend remains `373e5f6`, migration `0062`; frontend remains
  `f127621` on Vercel deployment `dpl_7aQhanG6EqR41hibnv8HdQfFxVDn`.
  No Git push, Actions run, Vercel deployment or migration was needed.
- With explicit user approval, created and activated `Development UAT — zero
  markup` through the signed-in Super Admin's development frontend Markup page:
  all B2B users, all airlines/routes, fixed BDT 0 per passenger, active version 2.
  Browser verified the active rule and available Edit rule action; changes use
  the canonical Rust business API and its normal version/audit records.
- The initial Python urllib diagnostic returned HTTP 403 from Cloudflare. A
  follow-up identified error 1010 (client signature restriction); an equivalent
  Login request without urllib's User-Agent from the same development server
  returned HTTP 200, `isSuccess=true` and a token. The earlier suggestion that
  server IP whitelisting was required was unsupported. Do not use urllib's 403
  as evidence that the deployed Rust client cannot authenticate.
- Enabled Triplover search with an optimistic version guard and append-only
  operator audit; retained a root-only backup of the original connection row.
  The canonical frontend does not yet expose supplier-configuration writes, so
  this was a scoped development database maintenance transaction. Firsttrip and
  Takeoff remain disabled. Triplover servicing/booking/ticketing remain disabled.
- The first frontend DAC–SIN search for 2026-09-30 dispatched to Triplover but
  hit the original 20-second supplier timeout, confirmed in Rust service logs.
  Raised only Triplover's timeout to 60 seconds with a second audited/versioned
  update (connection version 3). No booking, ticket or notification was created.
  Production unchanged.
- End-to-end retry succeeded through the stable development Vercel frontend and
  Rust supplier adapter: DAC–SIN, 2026-09-30, one adult, Economy returned 96
  displayed flight offers from Triplover. Rust search usage recorded success in
  43,265 ms; the earlier attempt failed at 20,015 ms. BDT 0 B2B markup remains
  active. Public health and authenticated canonical readiness passed, revision 3.
  Browser also showed a separate `Too many user searches` message in the booking
  assignee picker; booking/assignee selection was not tested or changed here.

### Production promotion preflight — 2026-09-21 (pending credentials)

- User authorized updating production environment configuration, then promoting
  the tested development release. User will supply new live Triplover credentials;
  do not substitute the UAT account. No production mutation or promotion yet.
- Fresh remote refs: backend `development` and `production` both `373e5f6`;
  frontend `main` is `b3c7eaf`, with five development commits through `f127621`
  ready for fast-forward promotion. No backend code deployment or schema change
  is needed for this release. Preserve unrelated local documentation changes.
- Production VPS `160.25.226.236` reports `APP_ENV=production`, release `373e5f6`,
  62 applied migrations, healthy internal live/ready endpoints, active API and
  notification services. Canonical authority is ready at revision 5, with matching
  runtime/durable pins, maintenance off and no pending/uncertain operations.
- Production supplier endpoints are placeholders, credentials are absent, all
  supplier controls are disabled, and no markup rules exist. Configure only the
  selected live supplier account after its values arrive; verify supplier login,
  search settings/pricing and health before publishing the frontend.
- Vercel remains linked to `babuas25/shapontravels-frontend`, production branch
  `main`, current production deployment `dpl_51FcVuBRXYDsNX3T5TfbuyJaPK9M` READY.
  Keep production API/identity configuration; no Preview pin or database copying.
- Frontend login-resilience, identity bridge/authority, seven Preview-readiness
  tests, optional-public-image checks and TypeScript checks passed on `f127621`.

### Production promotion completed — 2026-09-21 01:18 Asia/Dhaka

- User installed live Triplover credentials directly on production and restarted
  the API. Verified user endpoint `https://api.triplover.com` and search endpoint
  `https://apiv2.triplover.com`, successful HTTP 200 Login, token presence and valid
  expiry, without printing or exporting credentials/tokens. Currency is BDT.
- Before configuration changes, retained a root-only runtime environment copy,
  PostgreSQL dump and original supplier row under
  `/root/shapontravels-config-backups/production-promotion-20260920T191535Z`.
  Dump listing was checked; this maintenance did not perform a new restore drill.
- Created/activated `B2B default — zero markup` through the production Super Admin
  Markup page (all B2B users, all airlines/routes, fixed BDT 0, rule version 2).
  Enabled only Triplover search with 60-second timeout in a guarded audited
  database transaction, connection version 2. Firsttrip/Takeoff remain disabled.
  The user's environment booking/ticketing flags are true, but supplier database
  servicing/booking/ticketing controls remain false; these actions are not enabled
  by this search release and no booking, ticket or test notification was sent.
- Promoted frontend `development` to `main` by fast-forward in the authorized
  `babuas25/shapontravels-frontend` repository, commit
  `f127621199fc5c820c25c2e98ad27648a1912762`. Local development checkout and unrelated
  documentation edits were preserved; local main was synchronized.
- Vercel production deployment `dpl_AuNWCUizv65kodkcA6o9mucHn1UV` is READY at
  <https://shapontravels-frontend-oken9yazq-shapontravels.vercel.app/>; the stable
  <https://shapontravels-frontend.vercel.app/> alias points to this deployment.
  Compilation/TypeScript and all 43 static pages passed; inspected build logs
  contained no `[identity-verification]` messages.
- New production homepage correctly routed the existing signed-in Super Admin to
  the dashboard. Live DAC–SIN search for 2026-09-30, one adult Economy, displayed
  112 offers; Rust recorded Triplover success in 5,764 ms. The booking-assignee
  picker separately displayed `Too many user searches`; it remains an outstanding
  booking UI issue and was not part of the verified search path.
- Backend development/production remain the same `373e5f6` release with migration
  `0062`, so no backend push/Actions deployment or migration was needed. Production
  identity remains revision 5 with matching pins, canonical-ready and maintenance
  off. Public live/ready health checks passed; API and notification worker are both
  active. Vercel production identity/API configuration and development unchanged.
  Private readiness/deployment evidence: `.local/development-setup/production-promotion-*`
  and `.local/notification-rollout/production-promotion-*-20260921.json`.

### Development booking-assignee quota release — 2026-09-21 01:50 Asia/Dhaka

- Backend `development`: `00e26f9a21f7c1d1204f57c27f7e868c70f3d589`.
  [Actions run 35532785987](https://github.com/babuas25/shapontravels/actions/runs/35532785987)
  passed checks, PostgreSQL integration, release build and Deploy development.
  Verified this exact deployed SHA on `160.25.226.72`, active API service, and
  successful internal/public live and ready endpoints (`APP_ENV=uat`).
- Frontend `development`: `f3d64fe3a0193eb70970e95e5a48e0dbfc9294c0`, pushed to
  `babuas25/shapontravels-frontend` after backend readiness was verified.
  Vercel Preview `dpl_CqWKYUGDSuesGdWfaukv1EFVp1Kn` is READY at
  <https://shapontravels-frontend-notltwst6-shapontravels.vercel.app/>; stable
  development alias confirmed. Both authenticated build readiness gates passed,
  compilation succeeded, and all 43 static-page discovery steps completed.
- Canonical booking-assignee searches now use a PostgreSQL quota of 60 reads
  per actor per minute. The frontend no longer consults the legacy Supabase
  limiter for canonical searches, which caused false 429 responses when its
  production-mode deployment lacked Supabase configuration. Legacy mode has its
  own 60/minute assignee quota; booking submission limits are unchanged.
- Local validation passed: real-router disposable PostgreSQL business matrix
  with concurrent quota admission, actor isolation and expiry; auth unit check;
  frontend assignee, Rust holds/business, identity/resilience/authority checks;
  Preview readiness (7 tests), optional images, TypeScript and formatting checks.
- Stable development homepage routed the existing Super Admin session to its
  dashboard. Flight results rendered and the assignee picker showed the correct
  empty result, `No eligible users found.`, instead of `Too many user searches.`
  A separately authenticated Rust directory read returned HTTP 200 with zero
  eligible B2B owners. No test accounts, bookings, tickets or deliveries were created.
- No new migration or identity pin change: schema remains through `0062`,
  canonical rollout revision 3, runtime/durable pins match, maintenance is off.
  Real business notification worker remains intentionally inactive. Production
  code, runtime and configuration were not changed. Private release evidence:
  `.local/development-setup/assignee-*`.


### B2B development release preparation — 2026-09-21

The user requested committing and pushing all pending local backend/frontend
changes to their `development` branches. This backend change contains profile
fill-once enforcement, owner logo permissions, July–June trade-license renewal,
and rolling two-calendar-year CAAB renewal, plus onboarding and database tests.
No new migration or identity-pin change is required. The localhost endpoint
mismatch was resolved by rebuilding and restarting its existing launcher.

Rust formatting/Clippy, PostgreSQL onboarding/profile/document matrices, browser
journeys and frontend type checks passed before the push. Publish the frontend
only after the matching backend development pipeline and authenticated readiness
pass. The paired frontend release entry records the backend run and publication
verification. Existing production branches/configuration remain outside this
request; no manual VPS deployment or production migration is authorized here.

### B2B development release verified — 2026-09-21

- Backend `33b86607fecfaab6fc54097184777a8f999bc7ef` passed
  [Actions run 35536156525](https://github.com/babuas25/shapontravels/actions/runs/35536156525),
  including Deploy development. Exact deployed SHA, public health and authenticated
  canonical readiness were verified: migration `0062`, revision 3, matching pins.
- Frontend `0890fc915968e9c6a9096e7dcb22bf690140b9d3` completed Vercel Preview
  deployment `69xQsVXnZKuiFA3z7Z6w3nfnLzwd`. The stable development homepage
  resolved the existing Super Admin session to its dashboard. Real business
  notification delivery remains disabled. Private evidence:
  `.local/development-setup/b2b-release-20260921.json`.

### B2B production release verified — 2026-09-21 09:15 Asia/Dhaka

- The user explicitly authorized production promotion. Fast-forwarded backend
  `production` to `33b86607fecfaab6fc54097184777a8f999bc7ef`, then frontend `main`
  to `0890fc915968e9c6a9096e7dcb22bf690140b9d3` after backend verification.
  Both local working checkouts remain on `development`.
- [Actions run 35556026599](https://github.com/babuas25/shapontravels/actions/runs/35556026599)
  passed Rust/PostgreSQL checks, release build and Deploy production. Verified
  the exact deployed backend SHA on `160.25.226.236`, active API and HTTP 200
  from public live/ready endpoints. Authenticated canonical readiness is true,
  revision 5, with matching runtime/durable pins and maintenance off.
- No new migration, identity pin, environment or supplier configuration change.
  Schema remains `0062`. Deployment backup
  `/var/backups/shapontravels/db-20260921T031046Z.dump` exists; this release did
  not perform a new restore drill.
- API stop/start left the notification worker inactive because of its `PartOf`
  relationship. Verified dispatch owner `rust` and valid notification configuration,
  then resumed the previously active worker. Post-release checks confirmed it
  active with zero restarts. No synthetic notification delivery was requested.
- Vercel Production deployment `GgK6cG2j9cwHEBqG6ARV7bKZJPsg` succeeded for
  frontend `0890fc9` (GitHub deployment `6560939599`) at
  <https://shapontravels-frontend-by71n6rml-shapontravels.vercel.app/>.
  The stable <https://shapontravels-frontend.vercel.app/> homepage was checked
  in Chrome after deployment: the existing signed-in Super Admin reached the
  dashboard with its correct role and navigation, without a setup/readiness error.
  No new production account or B2B document upload was created for verification.
- Includes the prior booking-assignee quota fix and the tested B2B onboarding,
  Flight Search landing/sidebar, fill-once company information and document rules.
  CAAB replacement is available two calendar years after the last successful
  upload; trade-license renewal uses July–June and company logo changes remain
  unrestricted. Production B2B mutation journeys were not repeated with real data.
- Sanitized release evidence is retained privately at
  `.local/development-setup/b2b-production-release-20260921.json`.


### Registration feedback release — 2026-09-21

- User explicitly authorized development migration/push followed by production.
  Backend `96295fedf919049872ab7615fdd5fbb07ad5439d` passed
  [development Actions 35558568044](https://github.com/babuas25/shapontravels/actions/runs/35558568044)
  and [production Actions 35559249078](https://github.com/babuas25/shapontravels/actions/runs/35559249078),
  including checks, release build and the selected deployment job.
- Both VPSs run that exact SHA and migration `0063`. Internal/public health and
  authenticated canonical readiness passed; revisions remain 3/5, pins match,
  maintenance is off, and Clerk account writers remain disabled. Production
  backup: `/var/backups/shapontravels/db-20260921T040630Z.dump`. Migration preserves
  existing mail evidence and does not enqueue receipts for historical submissions.
- Frontend `0c42355354bfdfc561c754bba34e8133af4c8917` includes the submission success
  dialog, branded submission/approval emails and protected SMTP diagnostics.
  Development Preview `dpl_6dJioQPDiYVpiA4oufwRpTLzEKYd` is READY at
  <https://shapontravels-frontend-5oe0cw2wt-shapontravels.vercel.app/>.
  Both stable homepages resolved the existing Super Admin session correctly.
  No real application, approval or document change was created as a smoke test.
- Production-only Vercel SMTP settings and a dedicated identity mail credential
  were installed; Preview delivery remains disabled. Runtime environment backups
  are root-only under `/var/backups/shapontravels/env-before-registration-mail-*`.
  Production business notification ownership remains Rust and its worker is active;
  development real business delivery remains inactive.
- Initial identity delivery attempted the existing pending welcome message. All
  five attempts were definitely `not_sent`; the row is now `blocked` at its retry
  limit. Immutable attempt history is retained. Do not reset the row or blindly
  resend it. No submission/approval message was generated during verification.
- SMTP authentication succeeded locally and through the deployed, token-protected
  GET `/api/identity/mail` readiness endpoint. A stale-claim POST returned 409
  before any SMTP send. The readiness endpoint only verifies TLS/authentication;
  it does not prove provider acceptance of a complete message or inbox delivery.
- Found a sender mismatch: production authenticated as `no-reply@shapontravels.com`
  but the identity sender used `no-reply-uat@shapontravels.com`. Corrected the
  Production Vercel identity sender to the authenticated mailbox. This is a
  plausible cause of the initial rejection, not a confirmed provider diagnosis.
  A user-selected test recipient was requested for final message acceptance
  verification. Do not claim successful inbox delivery without that evidence.
- Private release/configuration evidence: `.local/registration-release/`.

- Final Production deployment `dpl_DTDg3UFSU5C7bkMzSG3Z2A8JarPs` is READY at
  <https://shapontravels-frontend-ac4wxsihy-shapontravels.vercel.app/> with the
  corrected identity sender. Stable production homepage/browser verification
  passed again. The authenticated SMTP readiness endpoint returned 200/ready;
  stale delivery claims still return 409 before sending.
- Re-enabled production identity autodispatch after this configuration correction.
  Authenticated readiness reports mail enabled, matching revision 5 and Clerk
  writers still disabled. API and business notification services are active;
  final public live/ready checks pass. The old welcome remains blocked and was
  not replayed. Full SMTP message acceptance/inbox verification remains pending
  the requested test recipient; future registration events use the corrected sender.


### Existing B2B partner approval correction — 2026-09-21

- Production investigation found an active B2B owner with an active agency and a
  pending application. The old review path accepted only customer identities and
  reported this eligibility mismatch as `IDENTITY_VERSION_CONFLICT`.
- Backend `eee82242e62e517739e2f1e1ed73e6be2c56a211` now permits an administrator to
  approve that retained application against its existing active agency and wallet.
  Identity/application/profile versions, ownership, scope and replay checks remain.
  Existing API access is preserved. Pending applications block separate manual
  agency provisioning; active owners awaiting review appear in the review queue.
- [Development Actions 35563217302](https://github.com/babuas25/shapontravels/actions/runs/35563217302)
  and [production Actions 35563996025](https://github.com/babuas25/shapontravels/actions/runs/35563996025)
  passed checks, build and their deployment jobs. An earlier check-only run was
  cancelled before deployment when the API-access preservation regression was added.
  Both servers report the exact final SHA, migration `0063`, matching pins,
  revisions 3/5, maintenance off and healthy internal/public live/ready endpoints.
  No new migration or environment changes. Production backup:
  `/var/backups/shapontravels/db-20260921T052715Z.dump`.
- Frontend `5178694f9449bc0c25acbbe4ccdb4857a7e5b9c0` refreshes a stale application
  without automatically retrying the decision, explains review for active owners,
  and prevents the pending applicant's B2B role-dropdown bypass. Preview
  `dpl_8xFNakpn3eEMiDXWpLmv9QfGJntY` and Production
  `dpl_GzDMY6vinJvQLRz7v2tyZkf59GDZ` are READY. Production URL:
  <https://shapontravels-frontend-7w0lju95w-shapontravels.vercel.app/>.
- Browser verified both stable homepages with the existing Super Admin session,
  then loaded the affected production application with its new active-owner notice
  and enabled Approve application button. The live review decision was left to
  the administrator; no production application, role or financial mutation was made
  as a test. Synthetic browser and PostgreSQL tests verified deliberate approval,
  stale/replay rejection and retained agency/wallet/balance/API access.
- Rust all-target Clippy, onboarding, full application/document and agency financial
  isolation matrices, frontend typecheck, Phase 5/onboarding, seven Preview readiness
  checks and optional-image checks passed. Production business worker was validated
  and resumed after activation; development real delivery remains disabled.
- A user-triggered production submission is now recorded as `application_submitted`
  / `sent` after the prior SMTP sender correction. This establishes provider
  acceptance of a real receipt, not inbox placement. The old blocked welcome is
  retained. Identity mail is enabled and Clerk account writers remain disabled.
- Private evidence: `.local/approval-release/`. Working checkouts remain on
  `development`; documentation-only release records use `[skip ci]`.

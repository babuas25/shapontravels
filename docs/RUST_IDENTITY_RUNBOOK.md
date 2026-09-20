# Rust identity — isolated setup, verification and activation gates

Updated 2026-09-20. Clerk owns login; Rust owns application authority. Production canonical activation is complete (revision 5), and development is active separately (revision 3). Production Rust business email/SMS delivery is enabled. See the [current deployment record and operating procedure](DEPLOYMENT_RUNBOOK.md) and [notification policy](BUSINESS_NOTIFICATIONS.md). The earlier [localhost cutover evidence](evidence/RUST_IDENTITY_LOCAL_CUTOVER_2026-09-17.md) and [restart instructions](LOCAL_PORTAL.md) concern a separate local target. Disposable fixture instructions below must not be applied to retained local or server databases.

## Start a disposable local environment

Use a separate checkout/copy with **no existing `.env` or `.env.local`**, a new loopback PostgreSQL database ending `_identity_test`, a designated disposable Clerk test instance and unused loopback ports. Do not repurpose the existing `portal-dev.mjs` saved UAT database: it retains bookings and supplier settings that need a separate matching review. Never point empty-database fixtures at an existing account or financial database.

Rust configuration, held in an external isolated environment file:

- `DATABASE_URL`: the new loopback fixture database; `APP_ENV=test`; `APP_BIND`: an unused loopback address.
- `PORTAL_IDENTITY_MODE=staged` and a new `stib_` + 43 base64url character bridge secret.
- `PORTAL_IDENTITY_CLERK_SECRET_KEY`: the disposable test-instance secret.
- For explicit first-operator setup only: separate `stio_` operator secret plus `PORTAL_IDENTITY_OPERATOR_ID`. Bootstrap requires the selected, verified existing Clerk subject and an empty registry.
- Keep `PORTAL_IDENTITY_PROVIDER_WRITES=disabled` until a designated disposable provider test is intended. `test` requires a test key and loopback `_identity_test` DB. `live` requires pinned canonical mode, a live Clerk key and HTTPS app origin; it is not selected by the localhost setup.
- Event/mail/asset transports are optional separate capabilities, described below. Do not copy live supplier credentials into this fixture.

Build and apply migrations explicitly using the Rust project. Launch the built binary from the isolated directory so dotenv cannot discover the normal project's credentials. Migration 0046 is required. The binary does not auto-migrate at startup.

Next configuration in the same isolated environment:

- `SHAPON_API_BASE_URL`: the fixture Rust loopback origin.
- `SHAPON_IDENTITY_MODE=staged`, `SHAPON_IDENTITY_AUTHORITY=rust-preview`, `SHAPON_WALLET_BACKEND=rust-preview`.
- `SHAPON_IDENTITY_BRIDGE_TOKEN`: the matching `stib_` token, server-only.
- Clerk publishable/secret keys for the **same disposable test instance**. Keep Supabase identity configuration empty.
- Use the development server for interactive preview. Production explicitly rejects `rust-preview`.

Run `npm run check:identity-local` after loading the external isolated environment. It checks mode, endpoint, test Clerk key shape, bridge authentication, current schema, bootstrap and recovery counts without printing secrets, creating a user, enabling writers or activating authority. Missing bootstrap is actionable setup, not an empty privileged account.

Bootstrap the explicitly selected test operator with `node scripts/identity-bootstrap.mjs user_SELECTED_TEST_ID` in the Rust repository and the isolated environment. Remove the operator capability afterward. Never bootstrap the first browser login or infer identity from email/metadata. The run performed for this delivery used synthetic providers; no actual Clerk bootstrap was performed.

Open the existing dashboard URLs for Users & Roles, Sub Users, Profile, B2B application, wallet, statement, saved passengers, API management and native Rust flight/hold flows. `/identity-status` reports connection/access/setup state; `/identity-recovery` reports durable work. Application access may be committed while provider effects remain pending; “pending” is not a failed local suspension.

## Provider and asset work

- Account create/invite/delete/effects: opt-in test writers only. Password is entered for dispatch and is never stored in operation intent or browser session storage.
- Webhook inbox: configure the disposable webhook to `/api/identity/events`, independent `stie_` capability and its signing secret. A failed durable acceptance returns a retryable error.
- Authentication email relay: preserve `/api/webhooks/clerk-email` and its existing Clerk signing-secret configuration if custom delivery is used. It preserves Clerk's authentication template and message ID. In preview, `SHAPON_IDENTITY_MAIL_DELIVERY=test` and loopback `SMTP_HOST` are mandatory. `user.created` does not send a second legacy welcome.
- Application mail: independent matching `stim_` tokens, loopback Next mail origin and disposable SMTP. Rust outbox/recovery owns welcome/invitation/activation delivery status.
- Wallet scheduler: `GET /api/cron/wallet-notifications` requires its existing `WALLET_NOTIFICATION_CRON_SECRET` (at least 32 characters). In preview it uses `stim_` against the dedicated Rust worker route; it never needs an interactive Clerk session. Directory expansion requires the exact current preparation lease. Preview email is loopback-only and SMS returns a visible test-transport failure without sending.
- Cloudinary: `SHAPON_IDENTITY_ASSET_MODE=test` and a cloud ending `identity-test` or `_test`. New identity documents are authenticated, no-overwrite objects with durable intent and current reference checks. Generic wallet asset uploads are also confined to that test cloud in identity preview.

Unknown provider, asset or delivery outcomes must be observed/reconciled. Do not resend because a lease or browser request timed out. Refresh durable status using the same operation ID. A stale fence/version requires a fresh read. Suspension revokes new access immediately; previously dispatched supplier/payment work must still settle through its original state machine.

## Repeat verification

Frontend: `npm run typecheck`, targeted ESLint, and the `verify-rust-identity*.mjs` scripts. Browser suites use actual components, synthetic loopback endpoints, blocked external requests and headless local Chrome. The account fixture uses the project's Tailwind theme. These fixtures are not evidence of a real provider login or delivery.

Rust: `cargo fmt --all -- --check`, `cargo test --lib --tests --no-fail-fast`, `cargo clippy --lib --tests -- -D warnings`. Ordinary tests deliberately skip database/live fixtures; explicitly run database matrices only against new disposable databases.

New matrices:

- `IDENTITY_BUSINESS_TEST_DATABASE_URL` → `cargo test --test identity_business -- --ignored --nocapture`.
- `IDENTITY_BOOKINGS_TEST_DATABASE_URL` → `cargo test --test identity_bookings -- --ignored --nocapture`. Runs realistic local search/reprice/booking fixtures with mocked suppliers, including unknown outcome/no-resend.
- `IDENTITY_BUSINESS_UPGRADE_TEST_DATABASE_URL` → `cargo test --test identity_business_upgrade -- --ignored --nocapture`. Requires a **clone** at migration 0044, not an empty database or a real target.

Previous lifecycle/profile/runtime matrices retain their documented environment variables and fresh-database requirements. Do not run any ignored live/UAT ticket tests as part of identity verification.

## Backup/restore rehearsal

`scripts/identity-restore-rehearsal.py` permits only loopback `_identity_test` sources and a new `identity_restore_*_identity_test` target. It never drops/overwrites a DB, activates authority or restores in place. It writes a custom-format dump plus non-PII evidence, compares every public table and sequence, and verifies the source is unchanged. Example with explicitly disposable names:

```sh
python3 scripts/identity-restore-rehearsal.py \
  --source postgresql://LOCAL_USER@127.0.0.1:PORT/SYNTHETIC_identity_test \
  --target identity_restore_REHEARSAL_identity_test \
  --pg-bin /absolute/path/to/postgresql/bin \
  --output /absolute/path/to/private/rehearsal-output
```

Use lowercase target/database names. The recorded rehearsal preserved all 77 table/sequence fingerprints at migration 0045 and successfully started the restored schema for a real HTTP readiness check. This does not back up or prove restoration of the actual deployment target.

## Phase 9 operator procedure

### Read-only target review

`POST /admin/portal-identity/preflight` requires the temporary **operator** capability (`stio_`), not the Next bridge. Its only input is the explicitly selected `clerk_user_id`. It checks current schema/bootstrap, the selected local Super Admin, 15 unresolved work queues (including business notification deliveries) and eight retained identity/business mapping issue categories. The mapping fingerprints cover 11 tables, limited to 100,000 rows each; larger reviews fail explicitly rather than silently truncate. The API does not call Clerk, resolve blocked work, import identities or mutate data. It does not prove that the selected Clerk login is currently usable.

`readiness` and `preflight` use PostgreSQL repeatable read/read-only transactions and no boundary rate-limit/audit writes. A SELECT-only DB role is supported for these two inspection endpoints. Unresolved work is reported by queue; one logical operation may occur in several queues. Pending invitations/financial requests and blocked mail require review, even when no uncertain provider call is present. Inspection errors never produce a clear report.

Once an actual target/operator is selected, supply its temporary operator capability through the external secret mechanism and run:

```sh
node scripts/identity-preflight.mjs \
  --origin https://SELECTED_RUST_ORIGIN \
  --operator user_SELECTED_OPERATOR \
  --output /private/review/NEW-review.json
```

The script does not load `.env`. HTTPS is required except for loopback HTTP fixtures. Output uses mode 0600 and refuses an existing path. It excludes raw subject IDs, credentials and personal fields. Add `--compare /private/review/PRIOR-review.json` to verify the same target/operator and detect changed mapping fingerprints. The artifact's SHA-256 detects corruption; it is not a signature or a substitute for independent review. Fresh remote responses must be within five minutes of the local clock. Review artifacts do not grant authority and there is no apply option.

Exit 0 means the selected local operator/mapping/backlog snapshot is clear, maintenance is enabled, and no compared mapping changed. Exit 2 preserves an actionable review with unresolved work, mapping drift or missing maintenance. Exit 1 means invalid configuration/response, failed request or output failure. **All outcomes retain `activation_ready:false` and `activation_performed:false`.** A clear database snapshot cannot prove backups, external writer shutdown, provider identity or target configuration.

### Coordinated maintenance preparation

The code now supports `SHAPON_IDENTITY_AUTHORITY=maintenance` in Next and `PORTAL_IDENTITY_MAINTENANCE=true` in Rust. Defaults remain legacy/false. Enable them only as part of the selected environment's coordinated deployment procedure. Both deployed targets were out of maintenance after their recorded 2026-09-20 rollouts.

Restart/drain **every** serving replica, scheduler and external worker before relying on the freeze. Next rejects all matched application/API/Server Action requests with 503, no-store and Retry-After. Rust rejects all ordinary routes, including legacy admin and machine APIs; only `GET /health/live` and the authenticated `POST` readiness/preflight endpoints remain. Rust's identity worker does no work and its cleanup worker is not started. `/health/ready` intentionally returns 503, removing the frozen instance from business traffic.

Maintenance is process configuration, not a durable canonical activation marker. It cannot stop old binaries, an external scheduler, direct DB access or already dispatched provider work. Retain and reconcile in-flight/unknown operations before making an activation decision. A graceful restart does not prove that every external request was never sent. The existing generic deployment helper expects `/health/ready` to pass and will stop a failed deployment; do not attempt to bypass that check to deploy a frozen service. Review the maintenance/restart sequence separately from normal release activation.

Local reproduction: `node scripts/test-identity-preflight.mjs`; fresh `IDENTITY_PREFLIGHT_TEST_DATABASE_URL` with `cargo test --test identity_preflight -- --ignored --nocapture`; then `scripts/test-identity-preflight-runtime.py` against that retained synthetic DB. The runtime harness requires a new output directory and only accepts loopback `_identity_test` targets. It starts the actual binary from an empty directory with synthetic credentials, verifies the HTTP CLI/freeze, allows a worker tick, and compares every table/sequence before and after. [Recorded evidence](evidence/RUST_IDENTITY_PHASE9_PREFLIGHT_2026-09-17.md).

### Pinned canonical rollout

Migration `0046_portal_identity_rollout.sql` and `src/identity/rollout.rs` provide the durable marker and fenced `activate`/`pause`/`resume` operator commands. A canonical Rust process requires `PORTAL_IDENTITY_MODE=canonical` plus a complete release/target pin (`PORTAL_IDENTITY_ROLLOUT_ID`, `PORTAL_IDENTITY_TARGET_ID`, `PORTAL_IDENTITY_ROLLOUT_REVISION`, `PORTAL_IDENTITY_BACKEND_RELEASE`, `PORTAL_IDENTITY_FRONTEND_RELEASE`). Every canonical request carries the matching rollout headers; an absent/stale marker, wrong release, old worker or old transaction fails closed. The marker is append-only and each transition writes immutable operator evidence.

`node scripts/identity-rollout.mjs plan --manifest MANIFEST.json --output NEW-plan.json` validates an explicit target/operator, the preflight artifact, mapping digest, backup/restore/writer-shutdown/configuration evidence and release pin without network access. `apply --plan NEW-plan.json --output NEW-result.json` is the only command that can request the Rust rollout endpoint; it performs no automatic retry and writes a private result. Activation requires maintenance, current preflight, matching runtime pin, an active selected Super Admin and explicit evidence. Pause preserves the active release; resume requires the next revision and explicit acknowledgment of any unresolved recovery. The script always records whether the result was confirmed and never deletes data or rolls back a provider action.

The full synthetic marker/pause/resume/fencing matrix is `tests/identity_rollout.rs` and passed on a new `identity_rollout_01_identity_test` database. The test verifies concurrent single-writer behavior, review drift blocking, stale headers, staged-runtime denial after activation, old-worker fencing, pause/resume revision rules and retained unknown mail. It does not authorize or perform a real target rollout.

The local retained-booking acknowledgment is deliberately narrow: `acknowledge_retained_bookings:true` permits initial activation only on loopback databases ending `_identity_test`, with zero mapping issues and no backlog other than bookings. Every retained booking must be an `outcome_unknown` older than five minutes; pending/recent bookings fail. Booking state/update time is included in the reviewed digest and checked under row locks. It never marks a booking resolved or bypasses account-deletion checks. The 2026-09-17 local activation records the user's explicit instruction to retain the unknown booking.

### Future target activation and recovery

Before actual activation, record the selected target/environment and Clerk operator, review immutable retained owner/client/booking mappings, take and restore a target backup, pause old account writers/workers, resolve uncertain operations, apply migrations and verify every authority consumer uses the same selected pin. Configure real provider/mail/storage secrets through the deployment secret mechanism, not chat. Then execute the approved non-destructive target smoke plan and record monitoring/rollback evidence.

The 2026-09-17 localhost evidence predates the separately completed server rollouts. Production and development now each have their own canonical selector and durable marker; their current recorded state is in the deployment runbook. Do not reuse localhost configuration, bypass pins or set preview identity mode on a deployed application. Activation does not authorize real account/data deletion or financial import.

After the first canonical authority write, switching back to stale Clerk metadata/Supabase permissions may restore revoked access. Pause mutations and recover forward; never drop identity tables or roll back only one side of a provider operation. Monitor readiness, unknown operations, event/mail backlog, current authorization denials and audit/store errors. Review unknowns explicitly instead of retrying external writes blindly.

# Shapon Travels API

Rust/Axum backend for the flight aggregation and booking requirements in [REQUIREMENTS.md](REQUIREMENTS.md).

[Client integration guide](docs/CLIENT_API_GUIDE.md): authentication, Search → RePrice → acceptance → Hold → Issue, money fields, retries and current capabilities.

## Local development

**For this workspace's existing Next frontend:** run `node scripts/portal-dev.mjs` from this Rust project, and keep `npm run dev` running in the sibling `shopontravels` frontend. This resumes the existing local portal database and serves Rust on `127.0.0.1:18081`. See [local portal commands and troubleshooting](docs/LOCAL_PORTAL.md). The general setup below uses a separate database/listener.

Install the Rust toolchain specified by `rust-toolchain.toml` and PostgreSQL 18. Create a dedicated `shapontravels` database. Copy `.env.example` to `.env` only on a new installation; preserve existing supplier credentials. Set `DATABASE_URL` for your own database. Environment variables take precedence over `.env`.

```sh
cargo run -- migrate
cargo run -- serve
```

The default listener is `127.0.0.1:8080`. Commercial Swagger is at `http://127.0.0.1:8080/docs/` and its downloadable contract at `/openapi.json`. These exclude Admin operations and schemas. Full definitions require a human Admin session at `/admin/openapi.json`. `/health/live` checks the process; `/health/ready` checks PostgreSQL and all migration checksums, with a bounded timeout. Neither endpoint validates supplier credentials. The application refuses to serve with missing or mismatched migrations.

Migrations run only through the explicit `migrate` command. The VPS deployment workflow uses a dedicated migration role and a separate least-privilege application role, with backup before migration. PostgreSQL audit triggers prevent normal update/delete/truncate, but database owners/superusers remain a trusted boundary.

Supplier connections start disabled in PostgreSQL. Configuration validates HTTPS URLs and execution flags without contacting suppliers. Environment capability flags do not prove account entitlement. Protected supplier controls and public Search/FareRules/RePrice are implemented within the coverage documented below.

## Current implementation

Foundation and authentication are verified. See [authentication contract](docs/AUTHENTICATION.md) for machine tokens, one-time bootstrap and admin/client management. Public Search selects equivalent offers by original supplier total, then applies markup. [Prebooking flow](docs/PREBOOKING_FLOW.md) covers selected-direction FareRules/RePrice, explicit local acceptance and customer-selected alternatives. Hold booking, public PNR status/deadline lookup and reconciliation evidence are implemented within documented coverage; an Admin reconciliation screen and audited manual outcomes are implemented locally. [Held-ticket Issue](docs/TICKETING_API.md) is implemented and verified on Triplover UAT; [Ticket reports](docs/TICKET_REPORT_API.md) are implemented and deployed; [Direct Issue](docs/DIRECT_ISSUE.md) is deployed for offline execution only; real Direct Issue and production commercial authorization remain disabled; [held cancellation](docs/CANCELLATION_API.md) is deployed for offline execution. `REQUIREMENTS.md` tracks completion per step; later evidence supersedes historical baseline notes.

On this workspace a PostgreSQL 18.3 development runtime has been built under the ignored `.local/` directory. Its cluster listens only on a private Unix socket. The helper sets the Rust PATH and dedicated local database URL without changing `.env`:

```sh
./scripts/dev.sh migrate
./scripts/dev.sh serve
# Run once to create your own human Super Admin; password is read without echo.
./scripts/dev.sh bootstrap-admin
```

Integration tests provision their identities only in disposable test databases. Bootstrap the working installation separately as needed.

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
# Use an EMPTY disposable database; this test writes and changes migration metadata.
TEST_DATABASE_URL=postgres://localhost/shapontravels_test cargo test --test database -- --ignored
```

`cargo test` intentionally excludes the database test unless explicitly requested. CI runs it against a fresh PostgreSQL service. Cargo.lock pins dependency versions; Swagger assets are vendored and served without a CDN.

HTTP logs contain a generated request ID, response status and duration. Headers, URL paths/query strings, bodies, configuration values and raw database errors are not logged. Each response has `x-request-id`. Secrets and `.local` development artifacts are excluded from Git.


## Approved pricing rules

Per-passenger supplier total plus exact markup is rounded half-up to two decimal places; discounts derive from that rounded selling total. Rounded passenger totals are multiplied by counts and aggregated. If audience/scope fallback finds no applicable rule, the internal pricing pipeline returns `PRICING_CONFIGURATION_ERROR`, without returning supplier fare. These rules are tested against sanitized production fixtures; admin markup management is now available; public Search uses this pricing pipeline after original supplier-total selection.

```sh
cargo test --locked pricing::tests
cargo test --locked --test production_fixtures
```

These tests run locally and never call production suppliers or issue tickets.

## Markup management

See [Swagger walkthrough and API contract](docs/MARKUP_API.md). Create a draft, activate it by ID/version, and use list/get/edit/status endpoints under the `Markup rules` Swagger group. Same-scope active duplicates return 409.

## B2B tiers and API Management

[Rust account and agency identity](docs/RUST_IDENTITY_REQUIREMENTS_PLAN.md) retains Clerk login and moves application identity to Rust PostgreSQL. Phases 0–8 are complete as staged/local implementation and verification, including lifecycle/recovery, profiles and approval, canonical business integration, account UI and isolated acceptance tests. [Phase 6–8 evidence](docs/evidence/RUST_IDENTITY_PHASE6_8_2026-09-17.md) records the tests and synthetic backup/restore rehearsal; the [runbook](docs/RUST_IDENTITY_RUNBOOK.md) describes setup and remaining activation gates. Phase 9 remains pending: actual target configuration, reviewed production activation code and live cutover are not complete. Defaults remain disabled; no live authority or provider activation occurred.

[Tier pricing](docs/B2B_TIERS.md) and [managed API access](docs/API_MANAGEMENT.md) were released on 2026-09-14 as `44fc47d`, including migrations 0021–0023. Commercial documentation excludes Admin definitions, which require a human Admin session. [Release verification](docs/evidence/API_MANAGEMENT_RELEASE_2026-09-14.md) records successful CI, database backups/migrations and production health/authentication checks. The sibling frontend remains committed locally and unpushed at the user's instruction; its production bridge and booking-price integration remain separate work.

## Initial public Search

[Portal prebooking](docs/PORTAL_PREBOOKING.md) is implemented locally in the existing frontend results design as a Search → RePrice → acceptance path with owner tier pricing. It requires the uncommitted migration 0024 and has not been deployed. Booking and wallet migration remain a later step.

[Saved passengers](docs/SAVED_PASSENGERS.md) now use local Rust storage through the existing frontend form and API contract. Migration 0026 retains owner-scoped profiles and immutable references; 103 original profiles were copied and verified locally. Booking/wallet integration remains separate.

`POST /api/Search` and `POST /api/FareRules` now use machine tokens. [Setup, examples and initial-release limits](docs/SEARCH_API.md). Activate your own markup rule and selected supplier Search controls before testing. Search now selects the lowest original supplier total for conservatively equivalent fares before markup; equal totals prefer Takeoff, Firsttrip, Triplover. Summary counts, airline/stops filters and price ranges now derive from retained public offers (B2B published gross; exact payable is in tier pricing/fareBreakdown). Broader equivalence and complex-fare coverage remain partial. Hold Book/status/reconciliation and configured held-ticket Issue exist; [Direct Issue](docs/DIRECT_ISSUE.md) is deployed with real execution disabled; [held cancellation](docs/CANCELLATION_API.md) is deployed with real execution disabled.

## Deployment

[Ubuntu 24.04 server setup](docs/SERVER_SETUP.md) and [GitHub Actions CI/CD](docs/CI_CD.md). Pushes to main and pull requests run checks and a Linux release build. VPS deployment starts only after server prerequisites and the `VPS_DEPLOY_ENABLED` repository variable are configured.


## Admin reconciliation screen

After applying migrations through `0014_booking_public_reference.sql` and starting the updated backend, open `/admin/reconciliation` on the same API origin. Sign in with an existing human Admin/Super Admin account. The Bengali screen lists unresolved bookings, performs read-only supplier status checks and records evidence-backed manual outcomes with history. No client token, code editing or Swagger input is needed. See [manual reconciliation](docs/BOOKING_API.md#admin-manual-reconciliation-and-screen) for boundaries. Released through the existing deployment workflow on 2026-09-11 (`bdd3ec7`), including migrations 0013–0014; no production supplier Hold/Issue is authorized.

## Held-ticket Issue

`POST /api/ticket/NewTicket` confirms a verified held booking with booking/ticketing permissions and a required idempotency key. `GET /api/bookings/{id}/ticket` retrieves saved ticket evidence. Migration 0015 adds the durable issue reservation; 0016 adds append-only saved-response verification. See [contract and execution limits](docs/TICKETING_API.md) and [UAT verification](docs/evidence/HELD_TICKETING_2026-09-11.md). Released on 2026-09-11 in `08a569e`, including migrations 0015–0016; production readiness and the three ticket endpoints were verified. Held-ticket dispatch now follows each supplier’s environment flag and database ticketing/servicing controls in both UAT and production, with wallet authorization required.

Local update, 2026-09-15: held-ticket issue now uses saved Book references without calling PNR. Explicit price acceptance, duplicate/Cancel protection and production restrictions remain. Missing or offset-free deadlines require supplier validation at NewTicket. The consistency review adds migration `0028_booking_pnr_observations.sql` to retain verified evidence after failed lookups and aligns surname-only ticket verification with Book. [Review and regression evidence](docs/evidence/TICKETING_WITHOUT_PNR_2026-09-15.md). This update has not been deployed; migration 0028 is required before serving it.

The portal's explicit **Refresh Status / Deadline** action now uses the existing receipt route and Rust PNR reconciliation helper. Page loads and the normal ticket flow do not automatically call PNR. Latest verified status/deadline is owner-scoped and preserved across failed refreshes; missing latest deadlines supersede Book values. [Contract](docs/PORTAL_HOLDS.md#explicit-statusdeadline-refresh) and [local verification](docs/evidence/PORTAL_PNR_REFRESH_2026-09-15.md). The new refresh action is local and not live-supplier verified.

## Ticket details reports

Owner-scoped live report lookup by booking UUID, STR reference or platform transaction uses accepted selling fares and verified ticket evidence. See [report API](docs/TICKET_REPORT_API.md). Released on 2026-09-11 as `cd3f2b5`, including migration 0017; production readiness and all three report routes verified.

## Uncertain ticket reconciliation

Read-only recovery from verified PNR and ticket reports has client and Admin endpoints, immutable evidence and no repeated Issue dispatch. See [ticket reconciliation](docs/TICKET_RECONCILIATION_API.md). Released as `15e8e58` on 2026-09-11, including migration 0018; production readiness and endpoint authentication verified.

### Staged portal identity

[Identity requirements and progress](docs/RUST_IDENTITY_REQUIREMENTS_PLAN.md) and [Staged API/setup](docs/PORTAL_IDENTITY_API.md) describe the dedicated identity bridge. It is disabled by default and does not change existing portal authority or perform live cutover.

[Phase 9 preparation](docs/evidence/RUST_IDENTITY_PHASE9_PREFLIGHT_2026-09-17.md) adds read-only target review, mapping drift detection and coordinated maintenance controls. The [runbook](docs/RUST_IDENTITY_RUNBOOK.md) distinguishes a clear review snapshot from production activation; canonical production selection and actual cutover remain outstanding.

B2B API clients can render the additive [reconciled fare breakdown](docs/FARE_BREAKDOWN_API.md): service charge/discount incorporates the tier adjustment, with accepted payable preserved.

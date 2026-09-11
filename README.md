# Shapon Travels API

Rust/Axum backend for the flight aggregation and booking requirements in [REQUIREMENTS.md](REQUIREMENTS.md).

## Local development

Install the Rust toolchain specified by `rust-toolchain.toml` and PostgreSQL 18. Create a dedicated `shapontravels` database. Copy `.env.example` to `.env` only on a new installation; preserve existing supplier credentials. Set `DATABASE_URL` for your own database. Environment variables take precedence over `.env`.

```sh
cargo run -- migrate
cargo run -- serve
```

The default listener is `127.0.0.1:8080`. Swagger is at `http://127.0.0.1:8080/docs/` and its downloadable contract at `/openapi.json`. `/health/live` checks the process; `/health/ready` checks PostgreSQL and all migration checksums, with a bounded timeout. Neither endpoint validates supplier credentials. The application refuses to serve with missing or mismatched migrations.

Migrations run only through the explicit `migrate` command. The VPS deployment workflow uses a dedicated migration role and a separate least-privilege application role, with backup before migration. PostgreSQL audit triggers prevent normal update/delete/truncate, but database owners/superusers remain a trusted boundary.

Supplier connections start disabled in PostgreSQL. Configuration validates HTTPS URLs and execution flags without contacting suppliers. Environment capability flags do not prove account entitlement. Protected supplier controls and public Search/FareRules/RePrice are implemented within the coverage documented below.

## Current implementation

Foundation and authentication are verified. See [authentication contract](docs/AUTHENTICATION.md) for machine tokens, one-time bootstrap and admin/client management. Public Search selects equivalent offers by original supplier total, then applies markup. [Prebooking flow](docs/PREBOOKING_FLOW.md) covers selected-direction FareRules/RePrice, explicit local acceptance and customer-selected alternatives. Hold booking, public PNR status/deadline lookup and reconciliation evidence are implemented within documented coverage; an Admin reconciliation screen and audited manual outcomes are implemented locally. [Held-ticket Issue](docs/TICKETING_API.md) is implemented and verified on Triplover UAT; [Ticket reports](docs/TICKET_REPORT_API.md) are implemented and deployed; Direct Issue, Cancel and production commercial authorization remain unfinished. `REQUIREMENTS.md` tracks completion per step; later evidence supersedes historical baseline notes.

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

## Initial public Search

`POST /api/Search` and `POST /api/FareRules` now use machine tokens. [Setup, examples and initial-release limits](docs/SEARCH_API.md). Activate your own markup rule and selected supplier Search controls before testing. Search now selects the lowest original supplier total for conservatively equivalent fares before markup; equal totals prefer Takeoff, Firsttrip, Triplover. Summary counts, airline/stops filters and net selling-price ranges now derive from retained offers. Broader equivalence and complex-fare coverage remain partial. Hold Book/status/reconciliation and UAT held-ticket Issue exist; Direct Issue and Cancel remain unfinished.

## Deployment

[Ubuntu 24.04 server setup](docs/SERVER_SETUP.md) and [GitHub Actions CI/CD](docs/CI_CD.md). Pushes to main and pull requests run checks and a Linux release build. VPS deployment starts only after server prerequisites and the `VPS_DEPLOY_ENABLED` repository variable are configured.


## Admin reconciliation screen

After applying migrations through `0014_booking_public_reference.sql` and starting the updated backend, open `/admin/reconciliation` on the same API origin. Sign in with an existing human Admin/Super Admin account. The Bengali screen lists unresolved bookings, performs read-only supplier status checks and records evidence-backed manual outcomes with history. No client token, code editing or Swagger input is needed. See [manual reconciliation](docs/BOOKING_API.md#admin-manual-reconciliation-and-screen) for boundaries. Released through the existing deployment workflow on 2026-09-11 (`bdd3ec7`), including migrations 0013–0014; no production supplier Hold/Issue is authorized.

## Held-ticket Issue

`POST /api/ticket/NewTicket` confirms a verified held booking with booking/ticketing permissions and a required idempotency key. `GET /api/bookings/{id}/ticket` retrieves saved ticket evidence. Migration 0015 adds the durable issue reservation; 0016 adds append-only saved-response verification. See [contract and execution limits](docs/TICKETING_API.md) and [UAT verification](docs/evidence/HELD_TICKETING_2026-09-11.md). Released on 2026-09-11 in `08a569e`, including migrations 0015–0016; production readiness and the three ticket endpoints were verified. Production supplier ticketing remains blocked.

## Ticket details reports

Owner-scoped live report lookup by booking UUID, STR reference or platform transaction uses accepted selling fares and verified ticket evidence. See [report API](docs/TICKET_REPORT_API.md). Released on 2026-09-11 as `cd3f2b5`, including migration 0017; production readiness and all three report routes verified.

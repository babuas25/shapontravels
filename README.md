# Shapon Travels API

Rust/Axum backend for the flight aggregation and booking requirements in [REQUIREMENTS.md](REQUIREMENTS.md).

## Local development

Install the Rust toolchain specified by `rust-toolchain.toml` and PostgreSQL 18. Create a dedicated `shapontravels` database. Copy `.env.example` to `.env` only on a new installation; preserve existing supplier credentials. Set `DATABASE_URL` for your own database. Environment variables take precedence over `.env`.

```sh
cargo run -- migrate
cargo run -- serve
```

The default listener is `127.0.0.1:8080`. Swagger is at `http://127.0.0.1:8080/docs/` and its downloadable contract at `/openapi.json`. `/health/live` checks the process; `/health/ready` checks PostgreSQL and all migration checksums, with a bounded timeout. Neither endpoint validates supplier credentials. The application refuses to serve with missing or mismatched migrations.

Migrations run only through the explicit `migrate` command. Production should use a dedicated migration role and a separate least-privilege application role; deployment policy remains pending. PostgreSQL audit triggers prevent normal update/delete/truncate, but database owners/superusers remain a trusted boundary.

Supplier connections start disabled in PostgreSQL. Configuration validates HTTPS URLs and execution flags without contacting suppliers. Environment capability flags do not prove account entitlement. Protected supplier controls are implemented. Public flight APIs remain pending supplier evidence and pricing/transaction decisions.

## Current implementation

Foundation and authentication are verified. See [authentication contract](docs/AUTHENTICATION.md) for machine tokens, one-time bootstrap and admin/client management. Supplier transport and established pricing arithmetic are implemented internally; see [supplier evidence boundary](docs/evidence/SUPPLIER_READINESS.md) for unfinished integration and required evidence. `REQUIREMENTS.md` tracks completion per step.

On this workspace a PostgreSQL 18.3 development runtime has been built under the ignored `.local/` directory. Its cluster listens only on a private Unix socket. The helper sets the Rust PATH and dedicated local database URL without changing `.env`:

```sh
./scripts/dev.sh migrate
./scripts/dev.sh serve
# Run once to create your own human Super Admin; password is read without echo.
./scripts/dev.sh bootstrap-admin
```

No real Super Admin or API client has been provisioned in the development database. Test identities exist only in disposable test databases.

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

Per-passenger supplier total plus exact markup is rounded half-up to two decimal places; discounts derive from that rounded selling total. Rounded passenger totals are multiplied by counts and aggregated. If audience/scope fallback finds no applicable rule, the internal pricing pipeline returns `PRICING_CONFIGURATION_ERROR`, without returning supplier fare. These rules are tested against sanitized production fixtures; admin markup management is now available; public Search integration remains pending.

```sh
cargo test --locked pricing::tests
cargo test --locked --test production_fixtures
```

These tests run locally and never call production suppliers or issue tickets.

## Markup management

See [Swagger walkthrough and API contract](docs/MARKUP_API.md). Create a draft, activate it by ID/version, and use list/get/edit/status endpoints under the `Markup rules` Swagger group. Same-scope active duplicates return 409.

## Initial public Search

`POST /api/Search` and `POST /api/FareRules` now use machine tokens. [Setup, examples and initial-release limits](docs/SEARCH_API.md). Activate your own markup rule and selected supplier Search controls before testing. Complex fares, complete lowest-fare deduplication and booking/ticket workflows are not finished.

## Deployment

[Ubuntu 24.04 server setup](docs/SERVER_SETUP.md) and [GitHub Actions CI/CD](docs/CI_CD.md). Pushes to main and pull requests run checks and a Linux release build. VPS deployment starts only after server prerequisites and the `VPS_DEPLOY_ENABLED` repository variable are configured.

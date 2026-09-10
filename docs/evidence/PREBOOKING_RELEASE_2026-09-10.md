# Prebooking release verification — 2026-09-10

## Scope

Release the current selected-direction FareRules/RePrice integration, supplier-confirmed nullable RePrice segment references, explicit alternative-offer acceptance, and rejected-quote protection. FareRules failures return `UPSTREAM_FARE_RULES_ERROR` without invalidating pricing eligibility. No automatic supplier switching or purchase is introduced.

Apply migrations 0010 (selected directions) and 0011 (reprice-required flag) before starting the new build. Both are additive; historical selection stays null and existing offers default to no classified revalidation failure. The established deployment helper backs up PostgreSQL, stops the API, applies migrations, starts the tested binary and verifies liveness/readiness. A failed migration requires inspection/fix-forward; no automatic rollback is claimed.

## Local release checks

- Code review covered request selection, supplier-reference forwarding, quote invalidation/recovery, acceptance and Book guards, plus regression tests and opt-in diagnostic harnesses.
- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings` passed.
- `cargo test --locked`: 22 unit, 3 HTTP/foundation and 6 production-fixture tests passed. Supplier network tests remained ignored.
- Disposable PostgreSQL integration suite passed against a newly created isolated database, including migrations, ownership, prebooking and booking mock coverage.
- Deployment shell syntax checks, all 5 deployment failure tests and `git diff --check` passed.
- Existing VPS service was active and readiness returned production/ready before release. Deployment uses the existing GitHub Actions pipeline.

## Verification boundary

Supplier evidence is in the separate contract and same-airline reports. This release does not claim live booking/ticketing validation, full canonical matching, complex scoped markup, branded/multiple-component fares or aggregate Search summaries.

## Deployment outcome

- Deployed application commit: `1233c5622e7f6f58c7027f9f41667e8ebc33b2cd`.
- [GitHub Actions run 34446909044](https://github.com/babuas25/shapontravels/actions/runs/34446909044): checks, Ubuntu release build and VPS deployment all succeeded.
- Deployment log confirms database migrations applied, runtime grants refreshed and the exact commit activated with liveness/readiness passing at 2026-09-10 06:54 UTC. The established helper requires successful backup before stopping the service and applying migrations.
- HTTPS `https://sendbox.shapontravels.com`: `/health/live`, `/health/ready`, `/docs/` and `/openapi.json` returned 200. Readiness reported production/ready, which checks the build's migration checksums.
- Served OpenAPI now documents selected directions, `FARE_UNAVAILABLE`, `REPRICE_REQUIRED` and `UPSTREAM_FARE_RULES_ERROR`; operation IDs remain unique. The pre-deployment snapshot lacked the new selection/error descriptions.
- Unauthenticated Search, FareRules, RePrice, acceptance and admin supplier requests returned 401. No authenticated supplier flow or booking/ticketing call was executed in this release smoke test.
- Local before/after OpenAPI snapshots and machine-readable checks: `.local/evidence/release-20260910/` (ignored by Git).

Next implementation milestone: aggregate Search summary/filter metadata from retained selling offers, with response-shape preservation and cross-supplier regression coverage. General canonical/complex-fare and booking/ticketing gaps remain separate.

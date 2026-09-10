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

Local checks do not establish deployment success. Record the exact CI run and post-deployment HTTPS checks after the pipeline finishes. Supplier evidence is in the separate contract and same-airline reports. This release does not claim live booking/ticketing validation, full canonical matching, complex scoped markup, branded/multiple-component fares or aggregate Search summaries.

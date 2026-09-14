# API Management release — 2026-09-14

## Reviewed changes

- Rust application: `44fc47d7e9cf5c174fc1bf3717f337095673f912` — B2B commission shares, immutable quote pricing, linked API-client controls, protected Admin documentation, and the previously verified explicit UAT-only cancellation adapter.
- Frontend: `7caaf8c` in the sibling `shopontravels` repository — API Management dashboard, server-side Rust integration, Clerk authority checks and local review tools. Frontend CI now runs the existing API Management authority/isolation checks.
- Normal server cancellation and real Direct Issue remain disabled. This release performs no supplier Book, Issue or Cancel test.

## Fresh local verification

- 64 ordinary Rust unit/foundation/fixture tests passed.
- Full disposable PostgreSQL integration passed in 41.11 seconds; the temporary database was deleted afterward.
- Rust formatting, all-target Clippy with warnings denied, five deployment-helper tests, shell syntax and diff checks passed.
- Frontend full ESLint, TypeScript, production build, API Management route/audit/network tests, and actual-component desktop/390px mobile browser tests passed.
- Prior real Clerk session verification was inspected: all checks passed and all three disposable users were deleted. Password/MFA/email journeys were not exercised by the sign-in-ticket test.
- The legacy frontend Supabase security script stopped at its missing-credentials assertion before remote access. It writes to Supabase and invokes retention, so it is excluded from this isolated release review. Live Supabase remains disconnected and untouched.
- Private environment files, local databases, backups, screenshots, tokens and credentials are excluded from both application commits.

## Local working database

The private local working database was at migration 20. A custom-format backup was created with owner-only access and its archive catalog verified before applying migrations 21, 22 and 23. All three are recorded successful. Evidence and the backup are retained under the ignored `.local/evidence/api-management-release-20260914T125016Z/` directory.

## Production release

The Rust application was pushed to `main`. [GitHub Actions run 34845637264](https://github.com/babuas25/shapontravels/actions/runs/34845637264) completed successfully: Rust/PostgreSQL checks, Ubuntu release build and VPS deployment. The established activation helper requires backup success before migrating, then applies grants, restarts and verifies readiness.

Independent public HTTPS checks at 12:58 UTC returned 200 for `/health/live`, `/health/ready`, `/docs/` and `/openapi.json`. Production readiness verifies embedded migration checksums through migration 23. Commercial OpenAPI serves both new pricing routes and excludes Admin paths/schemas/security definitions. Anonymous access to `/admin/openapi.json`, `/admin/api-clients`, `/admin/tier-policy` and a pricing lookup returns 401.

The private verification result is saved beside the local backup as `production-smoke.json`. No authenticated production client mutation, supplier request or private UAT data transfer was performed. A documentation-only follow-up commit uses `[skip ci]`; the deployed application remains `44fc47d`.

## Frontend connection

The user explicitly instructed not to push the frontend project. Commit `7caaf8c` stays local; no frontend push or Vercel configuration change was made. Frontend release is deferred under that instruction.

When a frontend release is requested later, Vercel access and the production Rust Super Admin credentials will be needed to provision a dedicated integration account and set `SHAPON_API_BASE_URL`, `SHAPON_API_ADMIN_USERNAME` and `SHAPON_API_ADMIN_PASSWORD` in Vercel. Secrets must remain server-only.

Frontend deployment and production authenticated Admin/B2B checks remain pending. Existing frontend flight booking and wallet calculations have not been migrated to the Rust tier-pricing engine.

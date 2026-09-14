# API Management integration

Backend released on 2026-09-14 as `44fc47d`, with migrations 0021–0023 applied to the local working and production databases. The sibling `shopontravels` Next.js/Clerk frontend is committed locally as `7caaf8c`; the user explicitly instructed not to push it. Frontend deployment and production bridge configuration remain deferred. See [release verification](evidence/API_MANAGEMENT_RELEASE_2026-09-14.md). Historical local-verification notes below describe the earlier implementation stage.

## Contract

| Endpoint | Access / purpose |
| --- | --- |
| GET `/admin/api-clients` | Admin: linked B2B list, `q`, exact `external_user_id`, `offset`; 50 rows and `hasMore` |
| POST `/admin/api-clients` | Superadmin: provision Basic, API-disabled client for unique immutable Clerk `external_user_id`; no credential generated |
| GET `/admin/api-clients/{id}` | Admin: current settings, version, commission share and credential existence |
| PUT `/admin/api-clients/{id}` | Admin: active state, permissions and per-minute rate limit; Superadmin required for tier or API enablement changes |
| GET `/admin/api-clients/{id}/history` | Admin: latest 50 client audit entries |
| GET/PUT `/admin/tier-policy` | Admin: global commission shares, optimistic policy version |
| GET `/admin/tier-policy/history` | Admin: latest 50 policy audit entries |
| POST `/admin/clients/{id}/reset-secret` | Existing Admin credential issuance/rotation contract; frontend permits Superadmin for other clients and eligible B2B owners for themselves |
| POST `/admin/clients/{id}/revoke-secret` | Existing Admin credential revocation contract; same frontend ownership restriction |

Client updates require `expected_version`, `tier`, `active`, `api_management_enabled`, `permissions` and `rate_limit_per_minute`. Stale writes return 409. API enablement requires Enterprise. Current commission share is returned as `commission_share_percent`; client settings use snake_case. List pagination uses `hasMore` and history uses `actorId`/`createdAt`.

The frontend supplies a fixed-action authenticated server facade at `/api/api-management`. It maps the fresh Clerk identity to the immutable API client ID; B2B users cannot choose another client. Backend integration credentials remain server-only. Actual Clerk actors are recorded in the frontend security audit; backend audit identifies the dedicated integration administrator.

## Access and lifecycle

Managed clients (non-null `external_user_id`) must be active, Enterprise and API-enabled to issue or use machine tokens. Downgrade clears API enablement. Suspension, downgrade or disabling access removes machine tokens; later re-enabling cannot revive them. Existing clients without an external identity retain their previous access policy.

The frontend's Users & Roles actions suspend the linked API client before B2B demotion, deactivation or deletion. External identity-console changes require corresponding API suspension. The Rust server does not query Clerk on every commercial request.

## Documentation

Public `/docs/` and `/openapi.json` include only `/api/*`, `/auth/token`, `/auth/me` and their reachable schemas. Admin endpoints, orphan Admin schemas and Admin security definitions are excluded from the JSON document itself. Full OpenAPI requires Admin authentication at `/admin/openapi.json`. Endpoint authentication and permissions remain mandatory independently of documentation visibility.

Swagger operates against the configured environment, not a simulated sandbox. Existing supplier execution controls are unchanged.

## Frontend setup

Use the sibling frontend's `docs/API_MANAGEMENT.md` for installation, roles and verification. Set `SHAPON_API_BASE_URL`, `SHAPON_API_ADMIN_USERNAME`, `SHAPON_API_ADMIN_PASSWORD` on the Next.js server after releasing the Rust migrations. The integration account needs `super_admin`; do not expose these variables through `NEXT_PUBLIC_`.

The interface has one API Management sidebar entry with client controls, commission settings, documentation and history. B2B owners receive configuration and commercial documentation only when eligible. This does not migrate the existing frontend supplier booking flow to the Rust booking engine.

## Verification

Regular Rust tests, full disposable PostgreSQL integration, all-target Clippy, formatting and diff checks passed. Tests cover provisioning, role authority, conflicts, immutable identity, disabled token exchange, existing-token invalidation, audit history and public/private OpenAPI separation. Frontend route/browser tests, production build and security-hardening verification passed. No live supplier mutation or production migration was performed.

## Local review follow-up — 2026-09-14

The sibling frontend's `verify:api-management-local` runner now exercises real frontend handlers and React components against this API over loopback HTTP and an isolated `api_portal_local_*` PostgreSQL database. `examples/local_api_management.rs` migrates/bootstraps only that dedicated local database, never reads `.env`, and has no supplier adapters. Clerk identity, frontend audit and frontend limiter are test doubles; the production app has no bypass. See the frontend's integration guide for commands and limitations.

The browser flow verified provisioning, tier/API enablement, persisted commission policy, secret issuance and token exchange, role isolation, documentation visibility, disable/downgrade invalidation, non-revival of old tokens and credential revocation. The local frontend Supabase connection was disconnected; no remote requests were made in this follow-up. Working/production API databases and live Supabase were not migrated or modified during this run.

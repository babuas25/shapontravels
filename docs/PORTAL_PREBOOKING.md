# Portal prebooking and staff fare review — local implementation

Current local entry point: **http://localhost:3000** → Rust **http://127.0.0.1:18081**. Earlier 3002 references below record the initial review. See [the follow-up flow review](evidence/PREBOOKING_FLOW_REVIEW_2026-09-14.md) for the latest backend corrections and verification.

The sibling `shopontravels` frontend uses Rust Search in its existing results design → RePrice → explicit price acceptance. Tier gross, commission and payable come from stored Rust pricing snapshots. Booking, ticketing, cancellation, wallet settlement and the legacy checkout are outside this step.

This change is uncommitted and undeployed. Migrations 0024 and 0025 are local only, exercised in isolated review databases. The working database and production database have not received it.

## Trusted frontend session

`POST /admin/portal-prebooking-sessions` requires a human **Super Admin** session and accepts `{ "external_user_id": "user_…", "staff_pricing": false }` (`staff_pricing` defaults to false). The frontend derives this value from a fresh, active Clerk B2B owner; it never accepts a client ID, tier, permission or identity override from the browser.

For a freshly authenticated active Clerk Admin or Super Admin, the trusted frontend sends `staff_pricing: true`. This creates/reuses a technical owner in `portal_staff_clients`, separate from any B2B membership. No account picker, client secret, tier or customer commission is assigned. Staff see published **gross = original base fare + taxes**, with `tier: null`, zero commission and gross as the internal sorting total. Gross does not use marked-up supplier net, supplier discounts or separate AIT. The staff fare table shows AIT separately and does not derive a service margin from this view. Supplier totals and gross per-passenger amounts come from immutable Search/RePrice originals using exact decimal passenger rounding, then passenger counts; staff pricing endpoints reconstruct this view for both existing and new snapshots. New B2B Search/RePrice snapshots also use original base + taxes as gross, with discount = (gross − marked-up supplier total) × configured tier share and payable = gross − discount (the legacy API field `commission` carries this discount). Historical accepted snapshots remain unchanged. Only authenticated staff portal sessions receive supplier amounts. External machine tokens are denied for these technical owners, even if an administrator resets a secret.

Staff can Search, RePrice and read fare rules. Acceptance is blocked in both the frontend handler and Rust token boundary. The frontend checks fresh Clerk authority on every action, so a demoted/inactive user cannot renew this server-only access. Existing tokens expire after five minutes and never enter the browser.

Supplier names are separate from supplier-cost visibility. The trusted frontend calls `POST /admin/portal-offer-suppliers` only for a freshly authenticated Clerk **Super Admin**. The Rust endpoint requires a Super Admin integration session and accepts `{external_user_id, offer_ids}` with 1–100 distinct offer IDs. It resolves the actor's active staff owner and reads each saved offer's supplier ID, returning only canonical display names keyed by offer UUID. Foreign/missing IDs fail the entire batch. Ordinary Admin, machine and portal tokens cannot call this endpoint; it is absent from public API documentation. No schema migration is needed. The frontend omits supplier names entirely from Admin/B2B responses, while both staff roles continue to see supplier/gross amounts.

The linked Rust client must be active, have audience `b2b`, and have `search:read`. All three membership tiers qualify. External API enablement and client secrets are unrelated to this internal connection; managed external API credentials still require an enabled Enterprise account.

The response contains a random `stp_` bearer token, `expires_in: 300`, and the client UUID. Store the token only on the trusted server. The database retains its SHA-256 hash, client and issuing administrator. The frontend creates a session per action; it does not expose or persist the token in the browser. Issuance is limited to 120 requests per minute per integration administrator; commercial requests also use the client's configured rate limit.

Portal tokens have only `search:read`, even when the linked client has additional permissions. The authentication boundary permits only:

- `POST /api/Search`, `/api/FareRules`, `/api/Reprice`, `/api/Reprice/accept`, `/api/pricing/offers`.
- `GET /auth/me`, `/api/pricing/offer/{id}`, `/api/pricing/reprice/{id}`.

Other commercial actions return 403. Admin endpoints do not accept portal tokens. All existing resource ownership and offer/price expiry checks remain in effect. Current tier policy is read on authentication; existing snapshots remain immutable. A new RePrice is needed to obtain changed pricing, and only the latest eligible price revision can be accepted.

Disabling the client, removing `search:read`, disabling the issuer or removing the issuer's Super Admin role deletes relevant portal sessions. Re-enabling an account does not revive old tokens. Shared row locks during issuance serialize it with suspension. Every subsequent request also checks current account and issuer status.

## Local verification

Ordinary Rust tests, Clippy with warnings denied, formatting and the complete disposable PostgreSQL suite pass. `tests/support/portal.rs` covers authentication, all tiers, external API gating, action restrictions, ownership, expiry, suspension, permission removal, issuer demotion and public-documentation isolation.

Build the offline HTTP example with `cargo build --locked --example local_prebooking`. Its only transport is a sanitized Search/FareRules/RePrice fixture; it cannot reach a real supplier. It binds `127.0.0.1:18082`, bootstraps an ephemeral administrator, and requires a new loopback database named `api_prebooking_local_*`. The frontend's [local runner](../../shopontravels/scripts/verify-rust-prebooking-local.mjs) exercises the actual Next route, React component, Rust HTTP API and PostgreSQL. Clerk identity is simulated explicitly.

See the sibling frontend's `docs/RUST_PREBOOKING.md` for the frontend bridge, repeatable commands, captured evidence and remaining integration limits.

The actual Next dashboard on port 3002 can use the separate API Management review server on 18081. Rebuild `local_api_management` and set `LOCAL_API_TEST_RESUME=1` with its existing loopback `api_portal_local_*` database to apply migrations while preserving review accounts and credentials. By default this example has no supplier adapters. Set `LOCAL_API_LIVE_READS=1` and `LOCAL_API_LIVE_SUPPLIERS=firsttrip,takeoff` to use the existing Rust supplier credentials for Search/FareRules/RePrice only. It reads only supplier keys from the Rust `.env`, keeps the explicit isolated database, and installs a wrapper that denies supplier writes and PNR/report operations. It does not mix the currently configured Triplover UAT inventory into live results. The fixture preview on 13002 remains separate.

## Actual dashboard live reads — 2026-09-14

The real Clerk dashboard on `http://localhost:3002` now connects to live FirstTrip and TakeOff reads via Rust on 18081. Only the isolated `api_portal_local_clerk_20260914` database was configured: both suppliers have Search/servicing enabled, booking/ticketing disabled, with 120-second supplier timeouts. The formerly empty markup table now has the same fixed BDT 500 per-passenger B2B markup used in the local fixture preview; it is local review pricing, not a change to production pricing. Existing memberships and credentials remain intact. The database was backed up locally before this setup.

Restart the backend from the Rust directory (port 18081 must be free):

```sh
LOCAL_API_TEST_DATABASE_URL=postgres://ashifbabu@127.0.0.1:55439/api_portal_local_clerk_20260914 LOCAL_API_TEST_RESUME=1 LOCAL_API_TEST_BIND=127.0.0.1:18081 LOCAL_API_LIVE_READS=1 LOCAL_API_LIVE_SUPPLIERS=firsttrip,takeoff cargo run --example local_api_management
```

The existing Next launcher continues to isolate Supabase, Redis, email and direct frontend supplier calls. Real supplier credentials remain exclusively in Rust. The live-read wrapper's no-write/PNR regression test and strict Clippy pass. No commits, pushes, deployment or remote database writes were performed.

Final browser verification: after refreshing the stale client bundle, the same DAC–SIN 2026-09-30 search showed 287 live offers (the preceding search returned 288). The first IndiGo offer displayed supplier BDT 27,639.07 and gross BDT 28,139.07; real RePrice confirmed both amounts in the original card. The local flight_bookings count remained zero.

Super Admin hold booking on behalf of a selected B2B owner uses separate, narrowly scoped portal controllers described in [PORTAL_HOLDS.md](PORTAL_HOLDS.md). Existing `stp_` sessions remain prebooking-only.

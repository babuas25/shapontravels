# Authentication implementation contract

Machine clients and human administrators have separate tables, token namespaces and authorization extractors. Supplier authentication is private to each adapter.

## Machine integration

`POST /auth/token` accepts exactly `{"client_id":"UUID","client_secret":"issued secret"}`. A successful response is `{"access_token":"stm_…","token_type":"Bearer","expires_in":1800}`. Supply it as `Authorization: Bearer …` to machine resources. `/auth/me` returns the server-owned identity, audience, agent assignment and permissions; these cannot be set during token exchange.

Tokens contain 32 random bytes encoded using base64url. Only their SHA-256 digests are stored; secrets use salted Argon2id. The database constrains token lifetime to exactly 1,800 seconds. Clients cache tokens, exchange credentials shortly before expiry, and coordinate concurrent renewal. There is no refresh token. A 401 may require a fresh exchange; failure with revoked/disabled credentials requires administrative intervention. Never blindly replay a booking/ticket mutation after a 401 or timeout.

Every authenticated machine request joins current client and credential state and reads current permissions/pricing context. Secret reset rotates immediately, retires the old credential and invalidates tokens minted from it; there is no overlap. Revoke disables the credential. Client disable deletes outstanding tokens so re-enable cannot resurrect them. Already accepted in-flight work is not represented as cancelled; later transaction workflows must enforce their execution gates.

All authenticated machine requests have a PostgreSQL-backed per-client, 60-second rate window (default 60 requests). Token exchange and admin login use **separate** quotas: 120 attempts/minute per source IP and 10 attempts/minute per identity, including successes. Blank/oversized credentials are rejected before consuming login quota. Exhaustion returns 429 `RATE_LIMITED` with `Retry-After: 60`.

Sources come from the socket peer. Forwarding headers are ignored unless that exact peer IP is listed in `AUTH_TRUSTED_PROXY_IPS`. A trusted proxy must overwrite `X-Real-IP` with one valid address; missing, duplicate or invalid values return 400 `INVALID_CLIENT_ADDRESS`. `X-Forwarded-For` is not trusted. Follow the [Nginx setup](SERVER_SETUP.md) when deploying behind a proxy; otherwise all callers behind it share its source bucket.

Password verification runs outside async workers with separate machine/admin capacity (two concurrent verifications each). Full capacity fails immediately with 503 `AUTHENTICATION_BUSY`, `Retry-After: 1`; there is no password-verification wait queue. Secret provisioning uses its separate four-slot hashing pool. These bounds do not claim complete network/DDoS isolation.

Book rechecks current client active state, booking permission and managed API eligibility after acquiring its reservation/authority lock. Direct-intent reservations also check ticketing permission. Held Issue rechecks its current execution grants under the reservation lock. A committed permission removal while Book is waiting returns 403 `CLIENT_BOOKING_DISABLED` with no new supplier dispatch. Credential validity is checked on authentication; work whose reservation has already committed is not cancelled by a later revoke.

## Human administration

Run `cargo run -- bootstrap-admin` against an already migrated database. The command reads the username and prompts for a password twice without echoing it. No password argument, default password, public registration, or plaintext credential file is used. Password length is 12–256 bytes. A transaction/advisory lock and singleton marker allow exactly one initial bootstrap; any existing administrator also closes bootstrap.

`POST /admin/login` accepts `username` and `password`; it returns a separate opaque `sta_…` session with a 30-minute lifetime. Use that session in the Bearer header for admin APIs. It is not a commercial machine token. `POST /admin/logout` revokes the session. There are no cookies or implicit browser credential forwarding. Browser integrations should hold sessions in memory and reauthenticate on expiry. Production must terminate HTTPS before exposing these endpoints.

Admin/Super Admin can provision, update, disable, reset or revoke API clients and change supplier controls. Only Super Admin can create administrators or change their active status. Self-disable is rejected and the last active Super Admin cannot be disabled. Disabling an administrator revokes their sessions permanently. Bootstrap, login/logout and management mutations are audited transactionally without secret/password values.

The resource ownership helper checks opaque resource ID, owner client and resource kind together, returning the same not-found result for absent and foreign resources. Search/booking/ticket endpoints must use it as they are implemented; end-to-end flight ownership remains part of those later checks.

## Responses and documentation

Swagger provides independent `machine_token` and `admin_session` security schemes. Token/secret responses and other HTTP responses use `Cache-Control: no-store`. Platform errors, including framework JSON/body/path/query rejections, have `{"error":"CODE"}`. Invalid JSON/schema input uses JSON 400/415/422 responses; oversized bodies return JSON 413; unknown routes/methods return JSON 404/405. Flight successes retain their documented envelopes. auth rejection is 401, permission denial 403, absence 404, conflict 409, rate limit 429 and database failure 503. Each response has a generated `x-request-id`.

Use separate least-privilege PostgreSQL roles for migration and runtime in deployment. Deployment secret rotation, expired-session/rate-bucket cleanup scheduling, external audit export, backups and retention remain deployment work; the service does not claim tamper resistance against a PostgreSQL owner/superuser.

## Implementation references

- [Argon2 password hashing API](https://docs.rs/argon2/0.5.3/argon2/)
- [Axum request-parts extractors](https://docs.rs/axum/0.8.9/axum/extract/trait.FromRequestParts.html)
- [Vendored Swagger UI integration](https://docs.rs/utoipa-swagger-ui/9.0.2/utoipa_swagger_ui/)

## B2B tier assignment

`GET /auth/me` includes the trusted `tier` (`basic`, `professional`, `enterprise`; null for B2C). New/existing B2B clients default to Basic under migration 0021. Existing client edits preserve tier. Only a human Superadmin can assign another tier through `PUT /admin/clients/{id}/tier`; human Admin can read it through GET. Machine credentials cannot assign a tier. See [B2B tiers and frontend pricing](B2B_TIERS.md).

`/auth/me` also exposes the current `commission_share_percent`, captured atomically with tier. Human Admin/Superadmin can configure the global shares via `GET/PUT /admin/tier-policy`; only Superadmin can assign a client tier. Existing tokens use current configuration on subsequent requests.

Ticket-management requests use distinct `ticket-management:read` / `ticket-management:write` permissions (migration 0052; no automatic grants). Only machine tokens are accepted on these routes. They recheck token/credential validity, current client permission and wallet-owner status under the authority barrier, then restrict every booking/request to the exact client ID as well as its wallet owner. These permissions authorize agency-to-staff requests and customer quotation decisions, never supplier execution or staff settlement.

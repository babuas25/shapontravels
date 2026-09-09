# Authentication implementation contract

Machine clients and human administrators have separate tables, token namespaces and authorization extractors. Supplier authentication is private to each adapter.

## Machine integration

`POST /auth/token` accepts exactly `{"client_id":"UUID","client_secret":"issued secret"}`. A successful response is `{"access_token":"stm_…","token_type":"Bearer","expires_in":1800}`. Supply it as `Authorization: Bearer …` to machine resources. `/auth/me` returns the server-owned identity, audience, agent assignment and permissions; these cannot be set during token exchange.

Tokens contain 32 random bytes encoded using base64url. Only their SHA-256 digests are stored; secrets use salted Argon2id. The database constrains token lifetime to exactly 1,800 seconds. Clients cache tokens, exchange credentials shortly before expiry, and coordinate concurrent renewal. There is no refresh token. A 401 may require a fresh exchange; failure with revoked/disabled credentials requires administrative intervention. Never blindly replay a booking/ticket mutation after a 401 or timeout.

Every authenticated machine request joins current client and credential state and reads current permissions/pricing context. Secret reset rotates immediately, retires the old credential and invalidates tokens minted from it; there is no overlap. Revoke disables the credential. Client disable deletes outstanding tokens so re-enable cannot resurrect them. Already accepted in-flight work is not represented as cancelled; later transaction workflows must enforce their execution gates.

All machine requests have a PostgreSQL-backed per-client, 60-second rate window (default 60 requests). Token exchange/admin login also have shared global protection (120 attempts/minute) and identity protection (10 attempts/minute), including successful attempts. No forwarded IP headers are trusted. Return 429 on exhaustion; the client should back off for 60 seconds. Password hashing runs outside async workers with concurrency capped at four. This implementation's global login cap is conservative and should be tuned with deployment traffic evidence.

## Human administration

Run `cargo run -- bootstrap-admin` against an already migrated database. The command reads the username and prompts for a password twice without echoing it. No password argument, default password, public registration, or plaintext credential file is used. Password length is 12–256 bytes. A transaction/advisory lock and singleton marker allow exactly one initial bootstrap; any existing administrator also closes bootstrap.

`POST /admin/login` accepts `username` and `password`; it returns a separate opaque `sta_…` session with a 30-minute lifetime. Use that session in the Bearer header for admin APIs. It is not a commercial machine token. `POST /admin/logout` revokes the session. There are no cookies or implicit browser credential forwarding. Browser integrations should hold sessions in memory and reauthenticate on expiry. Production must terminate HTTPS before exposing these endpoints.

Admin/Super Admin can provision, update, disable, reset or revoke API clients and change supplier controls. Only Super Admin can create administrators or change their active status. Self-disable is rejected and the last active Super Admin cannot be disabled. Disabling an administrator revokes their sessions permanently. Bootstrap, login/logout and management mutations are audited transactionally without secret/password values.

The resource ownership helper checks opaque resource ID, owner client and resource kind together, returning the same not-found result for absent and foreign resources. Search/booking/ticket endpoints must use it as they are implemented; end-to-end flight ownership remains part of those later checks.

## Responses and documentation

Swagger provides independent `machine_token` and `admin_session` security schemes. Token/secret responses and other HTTP responses use `Cache-Control: no-store`. Platform errors have `{"error":"CODE"}`; they are distinct from the future supplier-compatible flight response envelope. Invalid JSON/schema input currently uses Axum's 400/415/422 rejection; auth rejection is 401, permission denial 403, absence 404, conflict 409, rate limit 429 and database failure 503. Each response has a generated `x-request-id`.

Use separate least-privilege PostgreSQL roles for migration and runtime in deployment. Deployment secret rotation, expired-session/rate-bucket cleanup scheduling, external audit export, backups and retention remain deployment work; the service does not claim tamper resistance against a PostgreSQL owner/superuser.

## Implementation references

- [Argon2 password hashing API](https://docs.rs/argon2/0.5.3/argon2/)
- [Axum request-parts extractors](https://docs.rs/axum/0.8.9/axum/extract/trait.FromRequestParts.html)
- [Vendored Swagger UI integration](https://docs.rs/utoipa-swagger-ui/9.0.2/utoipa_swagger_ui/)

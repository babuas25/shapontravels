# Rust business notifications

Business email and SMS are delivered by `shapontravels-api notifications-worker`.
The API writes immutable recipient/event snapshots in the same PostgreSQL
transaction as the business event. No provider request runs inside that transaction.

## Recipient and channel policy

Only active B2B agency owners/members receive business messages. The agency and
its owner must also be active. Passenger/customer contacts and internal staff
are not notification recipients. Email/phone eligibility is checked again when
the worker claims a delivery. Deposits go to the agency owner, including the
deposit-request acknowledgement (previously an admin SMS).

| Event | Email | SMS |
| --- | --- | --- |
| Booking held, pending, in progress, unconfirmed, cancelled | Yes | No |
| Verified supplier expiry / imported booking expiry | Yes | No |
| Ticket issued, with persisted issue evidence | Yes | Yes |
| Deposit requested / approved | Yes | Yes |
| Deposit rejected | Yes | No |
| Refund, reissue, void lifecycle | Yes | No |

SMTP sender: `Shapon Travels <EMAIL_FROM_ADDRESS>`. The default address is
`noreply@shapontravels.com`; the current local configuration explicitly uses the
authorized SMTP mailbox `no-reply-uat@shapontravels.com`.
Reply-to: `support@shapontravels.com`. SMS provider: BulkSMSBD.
Email templates are rendered in Rust with escaped values, exact integer money
formatting, and links to the agent portal. Issued-ticket emails include a PDF attachment generated in Rust from the saved
passenger, ticket-number and itinerary snapshot, plus an agent portal link.
The bundled Noto Sans font is licensed under SIL OFL; no font/image network requests
run while sending.
Expiry notifications require stored supplier/import evidence. A local deadline
passing does not establish airline cancellation, and this worker makes no
supplier booking, cancellation, ticketing, or wallet mutation calls.

Sign-up, login, verification, password reset, invitations, welcome and account
role emails remain with Next.js/Clerk. Next.js business sender modules, business
cron routes, and legacy arbitrary-recipient email/SMS sharing are removed.
Clipboard and itinerary screenshot sharing remain available.

## Delivery semantics

PostgreSQL deduplicates `(source_key, channel, recipient)`. Atomic claims use
`FOR UPDATE SKIP LOCKED` and an attempt token. A successful SMTP/API response
means provider acceptance, not verified inbox/handset delivery. SMTP rejection
is retried after five minutes, up to three attempts. Network timeouts, malformed
SMS results and expired in-flight leases become `unknown`; they never automatically
resend. Acknowledgement retries use the original result without another send.
Snapshots and completed delivery/attempt history cannot be edited or deleted.

The worker checks the active identity rollout pin before each claim. No messages
are claimed while the rollout is suspended/mismatched. It stops claiming on
SIGTERM and lets a current attempt settle. Idle polling is once every five seconds.
One systemd service handles both channels, with at most two database connections;
Vercel has no business scheduler.

## Configuration and commands

Reuse the existing provider credentials on the VPS, outside Git:

- `SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SHAPON_TRAVELS_SMTP_PASSWORD`
  (or `SMTP_PASSWORD`), `SMTP_SECURE`, `SMTP_REQUIRE_TLS`.
- `EMAIL_FROM_ADDRESS`: a mailbox/alias authorized for the SMTP account.
- `BULKSMSBD_API_KEY`, `BULKSMSBD_SENDER_ID`, optional `BULKSMSBD_API_URL`.
- `APP_URL`: the HTTPS agent frontend origin.
- Existing `DATABASE_URL` and canonical `PORTAL_IDENTITY_*` rollout configuration.

`notifications-check` validates configuration without connecting or sending.
`notifications-activate` switches business dispatch to Rust in a transaction;
it refuses if an old business worker still has in-flight/preparing claims.
`notifications-worker` runs the delivery loop without opening an HTTP listener.

## Coordinated production rollout

Migration 0062 is additive and leaves dispatch assigned to the old owner until
explicit activation. Applying it alone does not send messages or replay backlog.
Prepare and test both project changes before the maintenance window:

1. Back up the database. Stop existing business workers/schedulers and allow
   in-flight provider attempts to settle; reconcile unknown results separately.
2. Deploy the tested API binary and migration using the existing deployment
   procedure. Deploy the frontend removal in the same maintenance window.
3. Update canonical release fingerprints/rollout pins with the existing identity
   rollout runbook. Do not bypass its readiness or writer-freeze requirements.
4. Copy existing provider credentials to the protected VPS configuration, set
   `APP_URL`, and run `notifications-check` as the service user.
5. Run `notifications-activate` with maintenance disabled and matching pins.
   Historical old queues are retained as history and never replayed by Rust.
6. Install/enable `scripts/deploy/shapontravels-notifications.service`, then verify
   service health, rollout readiness, new event counts and provider acceptance.

There is no automatic fallback to the removed frontend business sender. Stop the
Rust notification service to pause delivery. Do not point old workers at retained
queues as a rollback: that can resend historical messages.

## Validation

- `cargo test --locked --lib notifications::` exercises rendering and SMS policy.
- `BUSINESS_NOTIFICATION_TEST_DATABASE_URL=postgres://.../new_notification_test
  cargo test --locked --test business_notifications -- --ignored` requires a NEW
  empty loopback PostgreSQL database and never contacts providers.
- `tests/business_notification_provider_live.rs` contains ignored, explicit
  opt-in live smoke tests. Only run after the operator authorizes the exact
  `NOTIFICATION_TEST_EMAIL` / `NOTIFICATION_TEST_PHONE`; set
  `LIVE_NOTIFICATION_TESTS=authorized`. Each selected test sends one real message.
  Never rerun an unknown provider outcome without checking it first.

## Current rollout status (2026-09-20)

The local implementation has not been deployed or activated. The SMTP 553
sender/relay rejection was resolved by explicitly setting `EMAIL_FROM_ADDRESS`
to the authenticated mailbox `no-reply-uat@shapontravels.com` in the backend
`.env`. One Rust email smoke test using those backend credentials was accepted
by Zoho. This proves SMTP acceptance, not inbox placement. Using the originally
requested `noreply@shapontravels.com` still requires authorization of that alias.
The application loads `.env`; `.env.local` is not loaded automatically.

One earlier explicit SMS smoke test was accepted by BulkSMSBD. No additional
SMS was sent during this email fix. Live smoke tests are never part of CI.
The current local backend `.env` has SMTP settings; the business worker also
requires `APP_URL` and the BulkSMSBD configuration before activation. The email
smoke supplied a portal URL and unused SMS placeholders only in its subprocess;
it did not activate the worker or connect to a database.

The frontend production build and relevant account/notification checks pass.
Two existing static frontend checks (`verify-booking-rollout-controls` and
`verify-manual-booking-import`) also fail against the original Git HEAD: they
expect legacy authorization in routes already delegated to the Rust import
adapter. These are not successful validation gates for this change.

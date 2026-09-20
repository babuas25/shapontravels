# Rust business notifications

For environment selection, release order and post-deployment worker verification,
see the [deployment and maintenance runbook](DEPLOYMENT_RUNBOOK.md).

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

Production is deployed and active. Backend release `373e5f6ab4242bc1c860d66ecfd60627a0aa9657`
passed GitHub Actions run `35518720667`; its verified Ubuntu artifact is installed
on the production VPS. Migration 0062 is applied. The canonical rollout is active
at revision 5, with maintenance disabled and the temporary operator capability
removed. `shapontravels-notifications.service` is enabled, running without
restarts, and dispatch ownership is `rust`.

Frontend commit `b3c7eafeeb430187c0ece35d6b2863a6426fef0d` is pushed to the new
private frontend repository. Vercel deployment `dpl_51FcVuBRXYDsNX3T5TfbuyJaPK9M`
is READY and serves the production domain. Business sender modules and schedulers
are removed; the project has no cron definitions. Account emails remain separate.
The localhost:3001 production profile now uses revision 5; the signed-in
Super Admin account list was verified in the browser.

The existing frontend branded HTML header, footer and itinerary presentation
are reused in Rust (`assets/email/business.html`, `src/notifications/html.rs`).
Production uses `Shapon Travels <no-reply-uat@shapontravels.com>` with reply-to
`support@shapontravels.com`. SMTP TLS and authentication were verified from the
VPS without sending another message. The earlier Rust email smoke test was
accepted by Zoho and the user confirmed inbox receipt. Using the originally
requested `noreply@shapontravels.com` still requires authorization of that alias.
The application loads `.env`; `.env.local` is not loaded automatically.

The pre-migration production backup was restored to a new disposable database;
all 115 public tables/sequences matched and the source remained unchanged.
Production health and canonical readiness pass. The business delivery and
attempt queues are empty: no production booking/deposit was fabricated, and no
historical notification was replayed. Actual business-event inbox/handset delivery
will occur when eligible agent events are created; this deployment check did not
create those events. One earlier explicit SMS smoke test was accepted by BulkSMSBD.

Private deployment evidence is retained in `.local/notification-rollout/`.
Production GitHub auto-deploy has been restored to enabled.

Two existing static frontend checks (`verify-booking-rollout-controls` and
`verify-manual-booking-import`) also fail against the original Git HEAD: they
expect legacy authorization in routes already delegated to the Rust import
adapter. The production build, relevant account/notification checks and Rust CI
passed; these two pre-existing static failures were not represented as passing.

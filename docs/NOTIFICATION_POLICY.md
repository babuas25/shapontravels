# Approved notification policy — 18 September 2026

The user approved agency/requester-only automatic B2B emails, all four internal ticket-management roles, and the two designated deposit-alert numbers. They separately approved removing the automatic archive copy. The reference frontend was inspected as source code only; its database was not connected.

| Event | Email | SMS |
| --- | --- | --- |
| Invitation, account creation, welcome, B2B activation | Relevant account | None |
| Role change | Affected account | None |
| Authentication messages | Clerk-selected account address | No new custom SMS integration |
| Deposit submitted | Requester; active Super Admin, Admin and Accounts reviewers | 01921-232941 and 01989-715039 |
| Deposit approved | Requester | Agency's registered mobile |
| Deposit rejected | Requester | None |
| Native B2B booking status update | Assigned agency owner | None |
| Native ticket issued | Assigned agency owner, with ticket PDF | Agency's registered mobile |
| Refund/Reissue/VOID update | Agency owner and same-agency requester; all active Support, Accounts, Admin and Super Admin | None |
| Ticket-management financial exception | Internal staff only | None |
| Explicit manual sharing | User-selected recipient, only through an authorized sharing action | User-selected number, only through an authorized sharing action |

Automatic B2B delivery never selects a passenger/customer-contact email. Staff who create a booking on behalf of an agency do not become its customer-email recipient. Same-address recipients within each event/audience are deduplicated. Customer and internal audiences have separate templates. No automatic CC/BCC or archive address is added.

## Implementation

Migration 0051 adds a native recipient outbox for portal booking events, ticket-management updates and role changes. Event snapshots and recipients are persisted with the originating transaction. Ticket-issuance snapshots run after the transaction's wallet capture. Notifications never modify wallet balances or execute supplier operations.

The mail-only `/admin/portal-identity/notifications` worker checks the current rollout, user/agency eligibility and contact address before claiming. Interactive identity credentials cannot claim or complete a delivery. Concurrent claims are distinct. Confirmed failures have bounded delayed retries; unknown outcomes remain for review. Completion acknowledgement retries do not send again. These guarantees apply to the new native worker; existing Clerk authentication relay behavior is unchanged.

The frontend uses the existing reference email templates, native booking presenter, issued-ticket PDF and SMS templates. `/api/cron/portal-notifications` requires its scheduler secret. Wallet notifications retain their own authenticated scheduler and ledger-independent delivery tracking.

Identity mail queues now create recipient jobs only. Pending historical identity archive jobs are marked blocked by migration 0051. Historical wallet archive rows cannot be claimed, and new archive plans are rejected. The central mailer and identity mail receiver also remove/reject archive delivery.

## Local configuration

The approved admin SMS numbers and new internal worker/scheduler secrets are installed privately. They are not stored in this document or committed configuration. The local release keeps:

- `SHAPON_PORTAL_NOTIFICATIONS_ENABLED=false`
- `SHAPON_WALLET_NOTIFICATIONS_ENABLED=false`
- `SHAPON_IDENTITY_MAIL_DELIVERY=disabled`
- `SHAPON_IDENTITY_SMS_DELIVERY=disabled`
- `PORTAL_IDENTITY_MAIL_AUTODISPATCH=false`

This permits notification worker authentication without starting identity mail dispatch. No scheduler or real provider send is enabled by migration. Existing notification backlogs are retained for review.

Local activation completed at rollout revision **9**, schema **51**. Both liveness and readiness returned HTTP 200. Both scheduler routes returned 401 without credentials and 200 with `enabled: false` when authenticated. Wallet and flight tables matched the independently restored pre-migration backup. No pending identity archive jobs remain.

## Verification

- Disposable PostgreSQL suites: native recipient outbox/worker authority; identity runtime; wallet notification claims/retries; ticket-management journeys; canonical business authorization.
- Ticket-management recipient checks include the agency owner and all four active internal roles, excluding Media, suspended staff, passengers and archive copies.
- Native booking trigger tests verify owner-only delivery and a post-capture ticket snapshot.
- Frontend actual worker/template tests cover recipient selection, the two approved SMS numbers, disabled defaults, single provider dispatch and completion acknowledgement retries.
- A real SMTP protocol test sent an issued-ticket email with its PDF to a loopback-only disposable inbox. Exactly one recipient was accepted; no BCC was present. No external email/SMS service was contacted by this test.
- TypeScript, targeted ESLint, strict Rust Clippy and diff whitespace checks passed.

Private test/release evidence is under `.local/notification-tests/`, `.local/ticket-management-tests/` and `.local/notification-release/`.

## Remaining activation work

Obtain a designated test email and mobile number, verify controlled delivery with the configured live providers, review historical pending recipients/events, then enable and schedule the appropriate workers. Production rollout is separate.

This change does not add native automatic expiry detection or port legacy manual-sharing endpoints. Native booking notifications follow the persisted Book, ticket issue/verification and cancellation events currently supported. The reference's unconnected supplier-balance/manual-issuance alert remains separate operational work; no speculative alert is sent without a verified native event.

# Fresh wallet notification delivery checkpoint

Implemented 16 September 2026 across Rust `shapontravels` and the existing Next `shopontravels` frontend. This completes the staged notification implementation work, not full wallet cutover or actual recipient delivery UAT.

## Delivered

- Migration 32: recipient delivery records, immutable content/destination snapshots, immutable completed attempt history and idempotent operator actions. Financial event identity remains immutable.
- A separate Super Admin-authenticated Rust worker API atomically expands events/claims recipient jobs. No provider I/O is inside a financial or notification transaction.
- Existing request/approval/rejection email and request/approval SMS templates are reused, with gross-versus-net amount semantics and canonical agency partner phone lookup. Rejection has email only. Reviewer roles are active Super Admin/Admin/Accounts. Admin SMS recipients are explicitly configured afresh.
- Existing email archive copies have independent delivery records. Receipt uploads/private URLs are not included in notification content.
- The wallet SMTP adapter dispatches once, bypassing legacy automatic retries; SMS also dispatches once. Provider acceptance, confirmed failure and unknown outcomes are distinguished. Unknown is never automatically resent.
- Confirmed failures have bounded, delayed retries; finance can explicitly retry with reasons, expected attempts/generation and acknowledgement of uncertain outcomes. Repeated operation IDs replay the original result.
- Staff status UI masks destinations; the existing decision-email resend queues work without falsely claiming delivery. Missing decision-email contacts can be corrected and re-resolved while the suppressed snapshot remains in history.
- A dedicated bearer-authenticated Next scheduler endpoint, disabled-by-default delivery switch, work budget and runbook are present. No hosted schedule or delivery switch was activated.

## Executed checks

| Check | Result |
| --- | --- |
| `cargo test --locked` | 73 ordinary tests passed; private/live/DB opt-in suites remained gated |
| `wallet_notifications` PostgreSQL suite | Passed on fresh `wallet_final_notification_test`; concurrent claims, immutable plan/attempts, completion replay, retry delay, exhausted preparation recovery, stale tokens, crash-to-unknown, finance/support boundaries, missing-contact resolution and unchanged financial balances |
| Existing wallet PostgreSQL suite | Passed on fresh `notification_final_wallet_test`; kernel/workflow/report/runtime privilege invariants preserved |
| Actual Next → authenticated Rust → PostgreSQL suite | Passed on fresh `api_portal_local_wallet_notifications_03`; includes actual recipient plans/templates and mocked providers, lost completion response, failure retry, unknown review, escaped rejection reason, masked status, manual resend and no duplicate financial posting |
| `verify:wallet-notification-providers` | Passed actual SMTP/SMS adapter code against in-memory transports: one dispatch, no implicit BCC multi-recipient envelope, stable message ID, refusal versus timeout/unknown, configuration failure |
| Typecheck and changed-file ESLint | Passed |
| Rust clippy `--all-targets -D warnings`, fmt and both whitespace checks | Passed |
| Existing deposit approval/admin SMS verifiers and `verify:rust-wallet` | Passed |
| Existing `verify:email` | SMTP connection/authentication succeeded; no message sent |

The notification suites use synthetic recipients/assets and no live supplier or delivery-provider calls. The separate SMTP authentication check above did connect to the configured provider, without sending email. No real email/SMS, supplier booking, ticket, cancellation, old data import, deployment, or portal DB reset occurred. The test server on 18083 was started/stopped by the runner; running portal configuration remains unchanged.

## Practical limits and next work

SMTP/SMS acceptance is not confirmed inbox/handset receipt. There is no end-to-end exactly-once guarantee across a lost provider acknowledgement: such deliveries are held unknown for explicit review. Delivery claims expire after ten minutes; they do not become automatic retries. Recipient snapshots do not silently change destinations on retry.

Preview operators can inspect pending/preparing/failed/suppressed/unknown/sent status. Invalid/missing contacts remain visible. All live activation, browser interaction parity, hosting scheduler configuration, controlled recipient UAT and operational backlog checks remain rollout work. Refund/manual/ticket-management settlement consumers and native portal issue controls remain the next wallet implementation phase.

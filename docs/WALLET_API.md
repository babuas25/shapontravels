# Fresh wallet API

Implementation status: current source and isolated tests, 16 September 2026. The portal cutover is still in progress; see [phase status](FRESH_RUST_WALLET_PLAN.md).

## Client reads

Use the existing machine bearer-token mechanism and grant `wallet:read`. Existing active-credential, client/tier/access and rate-limit checks apply. Reads use an explicitly linked financial owner; they never create or credit an account.

| Method | Path | Result |
| --- | --- | --- |
| GET | `/api/wallet/balance?currency=BDT` | Own currency account's current available, held, total, status and version |
| GET | `/api/wallet/statement?currency=BDT&limit=50` | Own immutable ledger, interval opening/closing balances and next cursor |

Supported currencies: BDT and USD, scale 2. All money and ledger versions cross the API as **integer decimal strings**. For example, `"12345"` means 123.45 in the stated currency. Client display conversions must reject unsafe integers; do not coerce missing/invalid amounts to zero.

Illustrative Balance response:

```json
{
  "currency": "BDT",
  "scale": 2,
  "status": "active",
  "availableMinor": "800000",
  "holdMinor": "200000",
  "totalMinor": "1000000",
  "version": "17",
  "updatedAt": "2026-09-16T00:00:00Z",
  "asOf": "2026-09-16T00:00:01Z"
}
```

Statement accepts optional `from`, `to`, `type`, `limit`, `cursor` and `currency`:

- Default currency BDT; default limit 50; maximum 100.
- `from` is inclusive and `to` exclusive. Timestamps require an explicit timezone. Omitted bounds include the full ledger history; a supplied start must precede the end.
- `type`: `deposit`, `booking_hold`, `booking_confirm`, `hold_release`, `refund`, `manual_credit`, `manual_debit`.
- Each page returns `currency`, `scale`, `snapshotVersion`, `opening`, `closing`, `transactions`, `nextCursor`.
- Entries contain `id`, `sequence`, `type`, `amountMinor`, available/hold before, after and delta fields, `currency`, `scale`, nullable booking ID/reference, and `createdAt`. Supplier payloads, private notes and internal idempotency hashes are excluded.
- Rows are ordered by descending per-account ledger sequence. Keep the same currency/date/type filters with subsequent cursors. The cursor is bound to the account, filters and first page's upper sequence; new postings do not change that statement snapshot.
- Opening and closing balances use the complete ledger. A type filter only filters displayed rows.
- Empty accounts return zero balances and an empty list. All responses use `Cache-Control: no-store`.

Errors use the platform envelope `{"error":"CODE"}`. Invalid credentials: 401; missing scope: 403; no own account mapping: 404 `WALLET_NOT_CONFIGURED`; invalid cursor/filter/currency: 422; rate limit: 429; unavailable database: 503. A frozen account remains readable.

## Portal service boundary

`POST /admin/portal-wallet` is a private server-to-server bridge, authenticated with a Super Admin service session. It is not a browser-accessible generic wallet proxy. Its body contains a trusted `actor` and a typed `command`.

The Next bridge obtains the real current Clerk role and active status, resolves the canonical agency registry, and supplies `{external_user_id, role, owner, display}`. It never derives authority from the dashboard's development role preview. Staff have no personal financial owner. An owner/sub-user shares the verified agency account; a customer account is user-owned only when the frontend's existing retail feature gate permits access.

Implemented commands:

| Action | Purpose / authorization |
| --- | --- |
| `provision` | Explicit zero account creation; own owner or authorized finance-selected owner; immutable client-owner mapping |
| `summary`, `statement` | Own account, or permitted operational staff reads |
| `list` | `deposit`, `adjustment`, `accounts`, `ledger`, `booking_payments`; limit 1–100 and owner/kind-bound keyset cursor |
| `report_summary` | Staff-only complete per-currency balances, owner counts, ledger movement and request totals; independent of page limits |
| `setting` | Bank/MFS/branch management: Super Admin; sender accounts: own agency/user; version-checked changes and asset metadata |
| `deposit` | Own account only; stable request UUID, decimal major-unit amount, proof handle and method-specific fields |
| `adjustment` | Accounts/Admin/Super Admin; stable UUID, target account, credit/debit, decimal major amount and reason |
| `review` | Finance; different requester/checker; exact replay or conflict; approval and ledger posting commit together |
| `freeze` | Finance; frozen/active owner status, audited reason; freezing requires a reason |
| `notification_status` | Staff-only masked event and recipient status for a deposit |
| `notification_retry` | Finance-only explicit retry/resend, stable operation ID, reason and stale-state checks; unknown outcomes require acknowledgement |

Operational lists are live paginated views. Use Statement for a fixed historical financial snapshot. Report totals are SQL aggregates over all records in one read transaction; never recalculate complete totals from the first list page.

Settings and deposit amounts are validated again in Rust. MFS percentage is converted to basis points and snapshotted; net credit is gross minus fee rounded half-up to one minor unit. All five existing deposit methods are represented. Settings changes cannot rewrite previously submitted payment details. Approved requests and all ledger entries are immutable.

Deposit events are persisted with the request/decision. Migration 32 adds immutable recipient snapshots and attempt history. The separate private `POST /admin/wallet-notifications` worker supports `claim_event`, `prepare`, `fail_event`, `claim_delivery` and `complete`; only the Super Admin integration session can call it. Expired sending claims become `unknown`; automatic resend is prohibited for an ambiguous delivery. Confirmed failures retry at most three times per generation, five minutes apart. Existing templates, one-attempt provider adapters, staff status and finance resend UI are implemented. They remain disabled in the running portal. `sent` means provider acceptance; neither queued nor sent proves inbox/handset delivery. See [notification runbook](../../shopontravels/docs/RUST_WALLET_NOTIFICATIONS.md).

## Native NewTicket financial behavior

- Book remains unpaid. The six supplier request fields and the absence of an automatic PNR preflight are preserved.
- A new held issue needs an explicit client-owner mapping, a matching account/currency and the accepted immutable RePrice payable snapshot.
- Account/operation locks and database constraints commit the issue record and reservation together, before any supplier HTTP call. A missing mapping, frozen wallet or insufficient balance prevents dispatch.
- Verified ticket evidence captures the reservation once. Generic supplier failure/timeout/invalid evidence retains the money in reconciliation. There is no timer release or automatic NewTicket redispatch.
- Raw supplier outcome evidence commits before capture is attempted. If capture fails after ticket success, the response retains ticket evidence, returns 202, and exposes `payment.state` plus `requiresReconciliation: true`.
- The saved-ticket verifier and ticket reconciliation paths use the same capture finalizer. They can finish payment from persisted success without reissuing a ticket. The admin reconciliation queue includes unresolved financial settlement even when ticket evidence is already verified.
- Funded issued responses expose `payment: {state, operationId, required: true}`. 200 means verified ticket evidence and captured payment for new wallet-required held issues.
- Historical issues have no fabricated new charge. Their evidence is retained; the wallet requirement is never retroactively toggled. A historical ticket without a new captured operation has no new-ledger refund entitlement.

Direct Issue remains outside this held-ticket integration and retains its existing deployment gates. Portal issue controls, proof-bound non-issuance release and broader post-ticket/manual/imported bindings must be completed before full wallet activation.

## Verification

- `cargo test --locked --test wallet -- --ignored --nocapture` requires an empty loopback database named `*_wallet_test` in `WALLET_TEST_DATABASE_URL`.
- `cargo test --locked --test database -- --ignored --nocapture` uses an empty disposable `TEST_DATABASE_URL` and mocked supplier transports.
- Frontend `npm run verify:rust-wallet` checks exact UI units and real-role/shared-owner bridge logic without network access.

Never point these test commands at the portal or production database. Tests create synthetic financial records and intentionally exercise denied mutations and failures.


## Portal adapter checkpoint (16 September 2026)

The existing Next wallet routes and server-rendered owner loader now have an isolated `rust-preview` adapter. See [portal adapter behavior and limits](../../shopontravels/docs/RUST_WALLET_PORTAL.md). This is not a production cutover.

Private command additions:

- `request { id, kind: "deposit" | "adjustment" }`: own/shared agency request or staff-authorized request; null for foreign/missing/wrong-kind records. Authorization occurs before returning attachment handles.
- `list { kind: "ledger" }` also permits owners; the SQL scope restricts it to their wallet owner. Machine Statement remains separately redacted and snapshot-paginated.
- Deposit `attachmentDigest`: optional lowercase SHA-256 hex computed by the trusted portal from verified receipt bytes. If present, the digest replaces generated asset transport handles in the idempotency hash. Old callers without a digest retain their original hash behavior. A digest requires an attachment. The original stored handle survives a same-payload retry.
- Report balance groups include per-currency `frozenCount`. Native booking-payment rows include the recorded portal `booked_by_user_id`; an unknown human issuer is not inferred from an API client.

The portal never accepts a browser-supplied actor/owner as authority. Settings create requests require a stable UUID; updates/assets require the last loaded version. Receipt URLs are signed only after an authorized Rust read. Failed/ambiguous financial calls never fall back to Supabase.


## Portal held-ticket consumer

The scoped private `/admin/portal-holds/ticket` bridge uses native reservation/capture and saved-evidence recovery. Preview reads the mapped owner wallet and immutable accepted payable; it does not provision or spend. See [portal ticket wallet contract](PORTAL_TICKET_WALLET.md). Refund/manual/post-ticket and proof-bound non-issuance release are still separate pending consumers.

## Completed-unknown ticket hold release

[Non-issuance review](TICKET_NONISSUANCE_REVIEW.md) adds the private `/admin/portal-wallet/nonissuance` bridge and staged finance UI. Explicit supplier confirmation, immutable evidence and a different finance reviewer are required. Pending workers, deadlines and PNR status alone cannot authorize release. Receipt/API outcomes distinguish `not_issued` + `released` from unresolved and issued payment conflicts; NewTicket remains blocked after resolution. Refunds and pending-worker recovery remain separate work.

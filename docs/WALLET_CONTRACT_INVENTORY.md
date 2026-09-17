# Fresh wallet contract inventory

Status: Phase 0 implementation baseline, 16 September 2026. Existing UI endpoints remain the compatibility contract. Financial state moves to the Rust database; the existing agency registry remains an identity dependency, not a wallet store.

## Frozen policies

- Canonical financial owner: verified agency registry key (or enabled retail user identity); never unverified Clerk agency metadata. The frontend bridge uses the real Clerk role, not the development role preview.
- Agency owners and permitted sub-users share one owner/account; operational staff have no personal wallet. Link machine clients explicitly to that same financial owner.
- Fresh accounts start at zero. Provisioning is separate from external GET reads. No old financial records are copied.
- Currency scale: BDT and USD use 2; unconfigured currencies fail closed. Money is exact integer minor units, wire format is decimal integer text, UI conversion is range checked.
- Hold Book is unpaid. Native issue must reserve the immutable accepted payable before dispatch. Generic supplier errors do not authorize release.
- Freeze blocks new spending, not capture/release of existing reservations or legitimate credits/refunds.
- Lock order for booking money movement: client → booking/issue → wallet owner → currency account → reservation/posting. Review operations lock their request before the wallet; no reverse request lock from a wallet transaction.
- Deposit approval credits trusted gross minus the snapshotted rounded MFS fee. Deposit and adjustment creators cannot approve their own request. A generic refund cannot bypass issued-ticket management approval.
- The runtime uses the new wallet ledger exclusively after activation. Legacy direct/manual/imported/ticket-management callers require an explicit supported bridge; no passing Rust IDs into legacy booking RPCs.

## Existing portal route contract

| Route | Methods | Rust destination |
| --- | --- | --- |
| `/api/wallet/adjustments/[id]` | PATCH | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/adjustments` | GET, POST | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/admin` | GET, PATCH | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/company-bank-accounts/[id]/logo` | POST, DELETE | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/company-bank-accounts` | GET, POST, PATCH | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/company-mfs-accounts/[id]/assets/[kind]` | POST, DELETE | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/company-mfs-accounts` | GET, POST, PATCH | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/deposits/[id]/decision-email` | POST | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/deposits/[id]` | PATCH | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/deposits` | GET, POST | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/ledger` | GET | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/refunds` | POST | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/reports` | GET | Typed portal-wallet service; retain current response envelope |
| `/api/wallet` | GET | Typed portal-wallet service; retain current response envelope |
| `/api/wallet/saved-bank-accounts` | GET, POST, PATCH, DELETE | Typed portal-wallet service; retain current response envelope |

**Adapter checkpoint:** The routes above are now wired under isolated `rust-preview`, except refund, which explicitly returns preview-unavailable. Decision-email resend and masked recipient status/retry are now backed by Rust; the private worker and dedicated authenticated scheduler handle delivery. The owner server-rendered loader also uses Rust. Running portal activation remains pending. See [adapter evidence](evidence/FRESH_WALLET_PORTAL_2026-09-16.md).

## Existing storage operations and callers

| Operation | Current callers |
| --- | --- |
| `ensureWalletForOwner` | `app/api/flights/booking/issue/route.ts` |
| `ensureSessionWallet` | `app/api/wallet/route.ts`; `app/api/wallet/deposits/route.ts` |
| `listAccountLedger` | `app/api/wallet/route.ts` |
| `listWallets` | `app/api/wallet/ledger/route.ts`; `app/api/wallet/admin/route.ts`; `app/api/wallet/reports/route.ts` |
| `listAllLedger` | `app/api/wallet/reports/route.ts` |
| `setWalletStatus` | `app/api/wallet/admin/route.ts` |
| `beginBookingIssue` | `app/api/flights/booking/issue/route.ts` |
| `beginDirectTicket` | `app/api/flights/booking/route.ts` |
| `captureBookingReservation` | `app/api/flights/booking/route.ts` |
| `finalizeBookingIssue` | `app/api/flights/booking/issue/route.ts` |
| `readWalletForOwner` | `app/api/flights/booking/issue/status/route.ts`; `app/api/impexp/confirm-booking/route.ts` |
| `beginImportedBookingIssue` | `app/api/impexp/confirm-booking/route.ts` |
| `beginLegacyManualIssue` | `app/api/flights/booking/issue/route.ts` |
| `failBookingIssue` | `app/api/flights/booking/issue/route.ts` |
| `finalizeManualIssue` | `app/api/flights/booking/issue/route.ts` |
| `releaseReservation` | `app/api/flights/booking/route.ts` |
| `finalizeBookingCancellation` | `app/api/flights/booking/cancel/route.ts` |
| `markReservationForReconciliation` | `app/api/flights/booking/route.ts`; `app/api/flights/booking/cancel/route.ts`; `app/api/flights/booking/issue/route.ts` |
| `createDepositRequest` | `app/api/wallet/deposits/route.ts` |
| `listDepositRequests` | `app/api/wallet/ledger/route.ts`; `app/api/wallet/deposits/route.ts`; `app/api/wallet/reports/route.ts` |
| `findDepositRequest` | `app/api/wallet/deposits/[id]/route.ts`; `app/api/wallet/deposits/[id]/decision-email/route.ts` |
| `reviewDeposit` | `app/api/wallet/deposits/[id]/route.ts` |
| `createAdjustmentRequest` | `app/api/wallet/adjustments/route.ts` |
| `listAdjustmentRequests` | `app/api/wallet/adjustments/route.ts`; `app/api/wallet/reports/route.ts` |
| `reviewAdjustment` | `app/api/wallet/adjustments/[id]/route.ts` |
| `refundBooking` | `app/api/wallet/refunds/route.ts`; `components/dashboard/wallet/FinancialWalletManager.tsx` |

## Feature families and implementation ownership

| Family | Rust service | Verification |
| --- | --- | --- |
| Money kernel, ledger, idempotency | `src/wallet` | PostgreSQL concurrency and invariant tests |
| Client balance/statement | `src/wallet` public reads | Scope, cursor, identity and exact money tests |
| Portal ownership/read/admin | `src/wallet` portal API + Next adapter | Real-role and foreign-owner contract tests |
| Bank/MFS/sender configuration and deposits | Rust settings/deposit workflow | Form, fee and asset ownership tests |
| Review, freeze, adjustments, notification outbox | Rust administration workflow | Concurrent approval, requester/checker and delivery tests |
| Ticket reserve/capture/recovery | Native issue and verified recovery finalizers | Supplier write-count and crash-boundary tests |
| Refund/reissue/void consumers | Approved settlement bindings | Entitlement and duplicate settlement tests |
| Screens and reports | Existing components + Rust adapters | Browser parity and ledger aggregation tests |

This inventory records source dependencies; individual feature completion is tracked in FRESH_RUST_WALLET_PLAN.md.


**Portal native issue checkpoint:** `/api/flights/holds/ticket` now supplies scoped preview/issue/saved-verification commands to `/admin/portal-holds/ticket`, which invokes native NewTicket and the same wallet finalizer. Actual portal actor, owner account and accepted payable are retained. Receipt, dashboard/history and payment reports use saved ticket evidence. Proof-bound release and refund/manual/post-ticket consumers remain pending; see [the contract](PORTAL_TICKET_WALLET.md).

## Completed-unknown ticket hold release

[Non-issuance review](TICKET_NONISSUANCE_REVIEW.md) adds the private `/admin/portal-wallet/nonissuance` bridge and staged finance UI. Explicit supplier confirmation, immutable evidence and a different finance reviewer are required. Pending workers, deadlines and PNR status alone cannot authorize release. Receipt/API outcomes distinguish `not_issued` + `released` from unresolved and issued payment conflicts; NewTicket remains blocked after resolution. Refunds and pending-worker recovery remain separate work.

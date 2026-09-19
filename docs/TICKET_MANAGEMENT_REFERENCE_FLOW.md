# Refund, reissue and void reference flow

User direction (18 September 2026): preserve the UI and workflow in `/Users/ashifbabu/Documents/shapontravels-frontend`. That folder is a source-code reference only. Do not access or integrate its database, copy its database credentials, or import its ticket-management records. Implement persistence, authority and money movement in the current Rust system.

## Existing UI to preserve

`components/dashboard/ticket-management/TicketManagementWorkspace.tsx` is already identical in the reference and current frontend at review time. Retain its queue, filters, request detail, quotation presentation, customer decisions, assignment, settlement and history. Use the booking actions and request creation UX from the reference, including passenger/route selection and voluntary/involuntary request types. Do not replace it with a separate Accounts refund proposal screen.

## Lifecycle

1. The booking owner creates Refund, Reissue or VOID for eligible passengers/routes. Booking/action eligibility is enforced on the server.
2. Operational staff accept the incoming request into In Progress or reject it.
3. Staff publish a versioned quotation and customer confirmation deadline. The request enters Awaiting Confirmation.
4. The owner approves or rejects the current quotation. Expired or superseded quotations cannot be accepted.
5. Approved requests are assigned for financial settlement. Accounts staff require assignment; Admin/Super Admin can settle directly. Support can operate and assign but cannot move wallet funds.
6. After the manual supplier operation, authorized finance records settlement. The request remains Approved with the terminal outcome Refunded, Reissued or Voided; do not introduce a different customer-facing completion stage.
7. Preserve documented requotation and release/reopen paths, version conflicts, request idempotency, activity history, customer-safe redaction and notification events.

## Action-specific requirements

- Refund: quotation uses the selected tickets' user-payable entitlement minus airline/service fees. Settle only the accepted amount to the original charged wallet, consuming the matching ticket entitlement exactly once.
- Reissue: retain per-passenger fare-difference allocations, fees, customer approval, additional-payment reservation, capture/release, and predecessor/successor ticket numbers. Supplier execution remains manual in the reference flow.
- VOID: preserve the existing eligibility/time window and entitlement-minus-fees calculation, including credit, debit or no wallet movement as applicable. Preserve debit-hold release/reopen only under the supported not-performed conditions.
- Retain action restrictions for direct/imported/manual tickets from `lib/ticket-management/booking-actions.ts`; historical records without a corresponding current-ledger charge must never be assigned invented financial entitlement.

## Rust implementation sequence

1. Map the existing frontend request/detail/quotation/entitlement contracts and reference lifecycle rules to Rust schemas and commands.
2. Implement canonical actor/agency authorization, immutable accepted quotes, version/idempotency checks, request exclusivity and ticket entitlements.
3. Bind refund/void credits and reissue/void debit reservations to the existing wallet kernel; derive owner/currency/amount from trusted persisted records.
4. Route the existing frontend endpoints to Rust and preserve the existing UI contract. Keep legacy database access blocked in canonical mode.
5. Verify complete owner → operations → owner confirmation → finance journeys in disposable databases, including duplicate settlement, cross-agency access, over-refund, stale/expired quotes, parallel requests, lost acknowledgements and rejected/reopened operations.
6. Apply only validated additive migrations through the current local rollout process before enabling the new flow. No production activation or real supplier/financial operation is implied by implementation testing.

## Implementation status — 18 September 2026

Steps 1–6 are implemented and activated on localhost through migration 0050 and rollout revision 7. The original UI is connected to native Rust commands, and workflow/authorization tests pass in disposable databases. Existing local wallet and booking data was preserved. Notification delivery and staging UI journeys remain before production activation. See [local verification evidence](evidence/TICKET_MANAGEMENT_NATIVE_LOCAL_2026-09-18.md).

## Draft correction history

The unfinished standalone final-booking refund draft was removed after the user's clarification. Its migration was never applied and its endpoint was never started. No running wallet data or application behavior changed from that draft.

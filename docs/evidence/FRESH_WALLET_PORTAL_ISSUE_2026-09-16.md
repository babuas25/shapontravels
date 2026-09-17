# Fresh wallet portal issue checkpoint

Implemented and verified 16 September 2026 in Rust `shapontravels` and Next `shopontravels`. This milestone connects the existing held-booking receipt to native wallet-backed issue. Refund/manual/post-ticket settlement and proof-bound non-issuance release remain pending; this is not a completed wallet cutover.

## Implemented

- Private scoped preview/issue/saved-verification endpoint. Fresh real portal roles and active owner are checked; canonical agency must agree with immutable wallet mapping. The browser supplies neither owner identity nor supplier references.
- Exact accepted payable and owner account are rechecked inside native reservation. Preview does not provision or spend. Actual portal actor/role is recorded in the reserve and dispatch audit.
- One native issue reservation across double clicks and different operation UUIDs. Existing environment/supplier gates, wallet freeze, sufficient-funds checks and issue/cancel exclusion remain effective.
- Receipt ticket/payment projection, passenger identity-based ticket assignment, booking list/history and financial report status now reflect verified issue results. Financial reports retain the actual portal issuer and accept missing booking references without dropping the report.
- Stable browser operation UUID, review dialog, saved-status action and saved-evidence capture recovery. Unknown outcomes retain funds; no automatic NewTicket or PNR is introduced.
- The old confirmation-email Share button is disabled only for Rust receipts until its own adapter exists. Existing print/download remains present. Legacy receipt behavior retains its default sharing setting.

## Verification

| Check | Evidence |
| --- | --- |
| `cargo test --locked` | 73 ordinary tests passed; opt-in private/live suites stayed gated |
| Full PostgreSQL regression suite | `portal_ticket_regression_test`: migration/auth/booking/native issue/cancel/reconciliation and existing portal tests passed; simulated suppliers only |
| Final actual Next → Rust HTTP → PostgreSQL suite | `api_hold_verification_ticket_05`: passed, including real wallet report handler, canonical owner/role/origin restrictions, default-off portal gate, exact accepted amount, foreign account/currency/amount refusal, insufficient/frozen balance and actual actor attribution |
| Concurrent same booking | Different operation IDs result in one supplier dispatch, reserve and capture |
| Concurrent different bookings | Two bookings competing for one remaining balance produce one capture and one insufficient-funds refusal; earlier unknown hold remains intact |
| Lost HTTP acknowledgement | Explicit read/replay recovers the existing result; no supplier redispatch |
| Injected capture failure | Verified ticket proof survives, payment stays reserved; saved verification captures once without NewTicket/PNR |
| Unknown supplier outcome | Remains in reconciliation; failed saved verification and new request IDs neither release funds nor repeat issue |
| PNR accounting | Two deliberate mocked PNR refreshes in the setup; zero additional reads from ticket preview, issue, replay, verification or receipt reload |
| Receipt mapping | Hermetic test covers reversed passenger order, normalized letter case, multiple ticket numbers per passenger, and confirmed ticket with unsettled payment |
| Browser | Actual React controls with mocked supplier/identity on `api_hold_verification_ticket_03`: review dialog, Back leaves unpaid, explicit issue changes to Confirmed/Paid, correct ticket number and updated owner balance, reload retains result and has no second Issue control; no browser warnings/errors observed |
| Static/build checks | TypeScript, changed-file ESLint, Rust clippy with warnings denied, fmt and both whitespace checks passed; isolated React verification bundle built successfully |

Browser fixture: BDT 4,233.05 ticket payable; synthetic owner available balance changed from BDT 100,000.00 to BDT 95,766.95 after one confirmed issue. These are disposable test balances, not a real payment.

The runners started/stopped their own loopback servers. No real supplier Book/NewTicket/Cancel, email/SMS, historical import, running portal activation, live DB migration/reset, deployment or commit/push was performed. `.env.local` was unchanged.

## Remaining

1. Proof-bound non-issuance resolution and release; a timeout alone never proves non-issuance.
2. Approved refund/void/reissue and retained imported/manual settlement consumers, bound to original new-ledger charges.
3. Rust ticket confirmation-email sharing, complete UI/accessibility parity, notification scheduler/UAT and activation/rollback work.

Contract and operational behavior: [Portal ticket wallet](../PORTAL_TICKET_WALLET.md). Overall scope: [phase plan](../FRESH_RUST_WALLET_PLAN.md).

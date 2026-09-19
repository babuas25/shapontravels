# Portal held-ticket wallet bridge

Implemented 16 September 2026. This is a staged native issue connection, not a production wallet cutover or a supplier ticketing authorization.

## Contract

`POST /admin/portal-holds/ticket` requires the existing trusted Super Admin service session. Its body contains:

- `reader`: real portal `external_user_id` and `role` (`superadmin` or `b2b`).
- `draft_id`: a booked portal draft; private unsubmitted drafts are ineligible.
- `owner`: canonical `agency` owner resolved from the existing identity directory.
- `command`: `preview`, `verify`, or `issue`.

`issue` additionally requires `operation_id` (UUID), `account_id`, `amount_minor` (positive integer string) and `currency`. They represent the reviewed payment and must match the immutable accepted fare and mapped owner account. The browser cannot select another owner or supply supplier references. The service derives NewTicket's six fields from the saved booking.

The response contains local booking/owner IDs, currency, exact required minor units, `canSubmit`, `blockedReason`, the mapped wallet's available/hold balances and status, and a nullable ticket result with its separate payment state. The Next adapter range-checks integer strings before converting them for the existing UI.

## Authority and money

- Super Admin may issue on behalf of an active B2B owner; the owner may issue its own booked draft even when staff originally created it. Other portal roles remain disabled.
- Real Clerk role and owner status are checked by Next. Rust independently checks scoped draft ownership, active linked B2B client, `search:read`, absence of a staff search client, and the immutable client-to-wallet mapping.
- Preview creates no wallet and makes no supplier call. Missing mappings/accounts remain errors; staff must provision/link explicitly through the existing wallet setup workflow.
- Issue uses the same native service as `/api/ticket/NewTicket`. It rechecks client authority under the client lock, keeps issue/cancel exclusion, validates saved hold/deadline evidence, and commits the wallet reservation before dispatch.
- Portal review fields are checked again by the reservation helper. They cannot override accepted payable, owner, account, or currency.
- The reserve ledger records the actual portal actor/role; supplier finalization remains a system operation. The saved Book creator is unchanged.
- One durable issue per booking applies across different UUIDs, portal and API entry points. A different booking cannot reuse the same operation key.
- Native supplier/transport gates still apply in every environment. Production issuance requires the supplier ticketing flag, database ticketing/servicing controls and the same wallet/authority checks as UAT.

## Recovery

A received verified ticket and its wallet settlement are separate facts. The receipt can show a confirmed ticket with payment still held and a reconciliation message if capture fails. `verify` processes only saved ticket evidence and retries the existing financial finalizer; it never calls PNR/NewTicket. Repeated recovery cannot capture twice.

An ambiguous issue retains its wallet reservation. Preview and receipt reload do not resend. The UI keeps a stable submission UUID and shows saved-status/recovery actions after an uncertain HTTP result. A new submission is offered only after an explicit fresh preview says no issue operation exists; the server remains authoritative if an earlier request subsequently arrives.

No timer, generic supplier error, missing deadline, failed PNR read, or operator button in this milestone releases funds. Proof-bound non-issuance release and approved refund/manual/post-ticket settlement remain pending work.

## UI and rollout

Migration 0049 and the matching backend fix serialize wallet writes in one order:
identity authority barrier, domain rows (booking/ticket/review where needed),
wallet owner, account, then operation. Saved-ticket finalization and the SQL
posting function acquire the barrier before any wallet rows. Supplier I/O stays
outside these transactions; saved-proof recovery does not reauthorize the original
caller. This prevents concurrent reservation/capture from deadlocking. Apply the
migration and backend together; amounts, ledger history and replay rules are unchanged.

The existing receipt, print/download, passenger table and Booking Management list project verified ticket results separately from the original held Book state. Passenger ticket numbers are matched by identity; multiple numbers stay associated with that passenger. Saved history also reflects issued/pending ticket operations.

Portal issue controls and `/api/flights/holds/ticket` require `SHAPON_WALLET_BACKEND=rust-preview`. No running portal configuration has been changed. The Rust private endpoint independently enforces supplier flags, database controls and wallet authorization.

The old confirmation-email Share button is disabled on Rust receipts because its legacy booking lookup has no Rust adapter yet. Cancellation, Direct Issue, post-ticket management and automatic deadline refresh remain outside these controls. Explicit **Refresh Status / Deadline** still performs the separately authorized PNR read; **Check saved ticket status** and **Verify saved ticket result** do not.

Use the updated hold verifier in the frontend with a new `api_hold_verification_*` loopback database and the Rust `local_hold_verification` example. All supplier calls, identity/agency responses and funds are synthetic. See the [evidence record](evidence/FRESH_WALLET_PORTAL_ISSUE_2026-09-16.md).

## Completed-unknown ticket hold release

[Non-issuance review](TICKET_NONISSUANCE_REVIEW.md) adds the private `/admin/portal-wallet/nonissuance` bridge and staged finance UI. Explicit supplier confirmation, immutable evidence and a different finance reviewer are required. Pending workers, deadlines and PNR status alone cannot authorize release. Receipt/API outcomes distinguish `not_issued` + `released` from unresolved and issued payment conflicts; NewTicket remains blocked after resolution. Refunds and pending-worker recovery remain separate work.

## Safe outcome diagnostics — 18 September 2026

The nullable ticket snapshot now includes nullable `outcome` (`reason`, `nextAction`, `automaticRetryAllowed:false`). It uses the same safe classification as commercial Issue: fresh pending requests a saved-status check; stale pending, unknown supplier outcomes and outstanding settlement require support. Confirmed/settled and reviewed non-issued results have no unresolved diagnostic. Supplier response text is not exposed. This changes receipt guidance only; wallet state, issue admission, evidence verification and duplicate-mutation guards remain authoritative.

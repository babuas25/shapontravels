# Ticket non-issuance review and wallet release

Status: implemented for the isolated Rust wallet preview. No live cutover.

## What can be resolved

A native held-ticket Issue has finished locally with `outcome_unknown`, its exact wallet reservation is still reserved/reconciling, and no saved evidence indicates a ticket. The supplier must separately confirm that **no tickets were issued for the referenced booking/passengers, processing is complete, and this request cannot complete later**.

This is a **manual two-person attestation**, not automated verification of an airline's negative outcome. A URL and file fingerprint bind the human review to a document; they do not establish authenticity. Both operators must verify the supplier source, all six references and the passenger set. Keep the document in the restricted supplier case/document location; the application stores its URL, SHA-256 fingerprint, case reference and confirmation timestamp, not file contents. No URL is fetched by Rust.

- PNR `Booked`, empty ticket lists, a generic rejection, timeouts and elapsed deadlines alone never release funds.
- Normal Search → Reprice → Book → NewTicket is unchanged and makes no automatic PNR request. This review also makes no supplier calls.
- A `pending` worker cannot be released through this workflow, regardless of its age. There is no historical no-dispatch proof or timer-based recovery. Crash/worker resolution remains an operational recovery gap before Phase 5 exit.
- Captured charges, cancelled bookings, Direct Issue, refunds and partial releases are outside this endpoint.
- Release does not cancel airline seats or reopen NewTicket. The durable issue remains unique per booking; retry/new keys return its stored outcome.

## Operator flow

Under `SHAPON_WALLET_BACKEND=rust-preview`, the existing finance/deposits page includes **Ticket hold review** for Super Admin, Admin and Accounts. Support and owners cannot propose, decide or inspect this staff evidence surface.

1. Select an unresolved operation (the queue shows the oldest 100), inspect exact amount, owner, supplier references and passengers.
2. Obtain an explicit supplier confirmation after local Issue processing completed, within the previous 24 hours. Enter its case reference, restricted HTTPS evidence link and local confirmation date/time. Choose the document locally to calculate its fingerprint and submit the attestation. This creates no wallet posting.
3. A different authenticated finance operator opens the saved proposal, independently checks the supplier source/references/passengers, chooses the same document and records review remarks. The reviewer must explicitly confirm the terminal non-issuance basis before approving.
4. Approval rechecks eligibility and the full saved evidence snapshot. New observations, settlement, ticket proof or changed booking state invalidate approval. Reject the stale proposal and submit a fresh one after investigation; never edit evidence history.
5. Approval, exact reservation release, immutable decision and audit commit together. Retrying an identical proposal/decision returns its stored result. Conflicting reuse, self-review, file mismatch and duplicate settlement fail. A frozen wallet can finish this existing reservation.

An uncertain HTTP result should be followed by **Reload saved status**. No supplier Issue is resent. Preserve request identity if explicitly retrying an unacknowledged proposal.

## API and authorization

Browser: `POST /api/wallet/nonissuance`. The Next handler requires the current active Clerk identity, actual finance role, same origin and strict input. Browser-supplied actor, owner, amount and currency are rejected. Role preview cookies are not authority.

Private Rust bridge: `POST /admin/portal-wallet/nonissuance`, authenticated with the existing Super Admin service session and a server-derived `actor`. Commands:

| Action | Input | Result |
| --- | --- | --- |
| `queue` | None | Oldest 100 unresolved issue/payment cases and actual actor ID |
| `inspect` | `issue_id` | Saved references/passengers, exact reservation, snapshot hash, eligibility and up to 100 proposals/decisions |
| `propose` | Stable UUID `id`, `issue_id`, `snapshot_hash`, `evidence` | Append-only attestation; no funds move |
| `decide` | `proposal_id`, `decision` (`approved`/`rejected`), `evidence_sha256`, `remarks` | Independent decision; approval releases only the stored reservation |

`evidence` has `confirmation_reference`, `evidence_url`, `sha256`, `confirmed_at` (RFC3339 UTC) and `supplier_confirmed_no_ticket_and_processing_complete: true`. All input types reject unknown fields. Amounts originate exclusively from the original wallet operation; wire amounts remain integer minor-unit strings.

Relevant conflicts: `WALLET_SELF_APPROVAL_FORBIDDEN`, `NONISSUANCE_EVIDENCE_CHANGED`, `NONISSUANCE_NOT_ELIGIBLE`, `NONISSUANCE_CONFIRMATION_MISMATCH`, `NONISSUANCE_ALREADY_REVIEWED`, `IDEMPOTENCY_KEY_REUSED`. Incomplete/old/future confirmation returns `INVALID_SUPPLIER_CONFIRMATION`.

## Storage and concurrency

Migration 33 adds immutable proposals/decisions, one approved decision per issue, a positive-evidence observation view and a shared ticket outcome projection. Deferred database constraints require an independent approved decision for any native ticket `hold_release`, and require that decision to have the matching operation/ledger release. Financial kernel balance/ledger invariants remain in force.

Lock order: booking → issue → proposal → wallet owner → account → operation. No transaction holds these locks over supplier HTTP. NewTicket's worker, saved verification and ticket reconciliation serialize on the same issue row. Evidence is checked again after lock acquisition. Proposal/decision insertion, posting and audit are atomic.

Raw Issue evidence is never rewritten. The projected `not_issued` state is distinct from the original `outcome_unknown` record. Machine ticket status/replays return HTTP 200 with `state: not_issued`, `payment.state: released`, `requiresReconciliation: false`, `canIssueAgain: false`. Booking receipt/list/report views retain the held airline booking and display released funds.

### Contradictory evidence after release

Manual supplier confirmation can be wrong. A later positive PNR/report signal reopens the unresolved projection; it does not by itself assert verified issuance. Verified ticket proof always takes precedence and is retained: the ticket projects `issued` with payment `released`, HTTP 202 and `requiresReconciliation: true`. The finalizer records `wallet.nonissuance.contradicted` once. No second release or silent debit occurs. Investigate with the supplier and use a separately approved financial resolution; automatic corrective charging is not implemented here.

## Verification and activation

See [local verification evidence](evidence/FRESH_WALLET_NONISSUANCE_2026-09-16.md). Supplier calls in tests are mocks, funds and identities are synthetic, and databases are disposable. Migration 33 has not been applied to the real portal database. Existing environment files, live supplier gates and wallet activation remain unchanged.

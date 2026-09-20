# Deposit approval, reversal and booking payment reporting

Implemented in migration `0059_wallet_deposit_controls.sql` and the Rust wallet/portal adapters. These changes do not call suppliers or payment providers.

## External payment deduplication

Deposit requests can still be submitted and rejected independently. On approval, the database claims the external payment identity exactly once, across request UUIDs, wallet owners and currencies. The claim and credit commit or roll back together. Replaying an already approved request returns its original result.

Payment identity rules:

| Method | Identity |
| --- | --- |
| Mobile | Provider, destination account number, transaction ID; independent of submitted date, amount and receipt |
| Bank / bank transfer | Destination account number, deposit date, reference; both methods share a namespace |
| Cheque | Destination account number, issuing bank, cheque issue date, cheque number |
| Cash | No external transaction identity; retain request-level idempotency and independent review |

Identifiers are trimmed and case-normalized. Snapshotted account numbers are used so recreating a setting does not make the same payment new. Historical settings without an account number fall back to the saved setting ID. Bank dates remain part of the key because references can be reused on different dates. A payment's amount, wallet owner and receipt are deliberately not identity fields.

Approval of an already credited payment returns HTTP 409 `DEPOSIT_PAYMENT_ALREADY_CREDITED`. A rejected request never consumes the payment identity. Reversing an approved deposit does not free its identity for reuse.

## Deposit reversals

The portal's **Adjustments → Reverse a deposit** form submits a separately reviewed debit. It selects an approved deposit and derives its wallet and currency on the server. The amount is capped at the deposit's **net credited amount minus previously approved linked reversals**. Pending reversals do not reserve funds or consume this cap; competing approvals recheck it atomically.

Private Rust bridge command:

```json
{
  "action": "reverse_deposit",
  "id": "<stable new request UUID>",
  "deposit_id": "<original approved deposit UUID>",
  "amount": "35.00",
  "reason": "Correct duplicate deposit"
}
```

The resulting request remains `kind=adjustment`, `adjustment_type=debit`, with immutable `reversal_of`. Review uses the existing `review` command and requires a different financial operator. Frozen wallets and insufficient available funds reject approval without creating a posting or changing request status. Partial reversals are supported; the cumulative approved amount cannot exceed the original credit.

The original deposit stays approved and unchanged. Each reversal posts an ordinary `manual_debit` with its original deposit and original ledger entry IDs in metadata. Existing transaction-type consumers remain compatible. Request reads expose `reversal_of`, `reversal_deposit_ref` and, for approved deposits, `reversible_amount` in exact minor-unit text. The portal converts this amount at its existing checked integer boundary.

Errors: HTTP 409 `INVALID_DEPOSIT_REVERSAL`, `DEPOSIT_REVERSAL_EXCEEDS_CREDIT`, `WALLET_SELF_APPROVAL_FORBIDDEN`, `WALLET_FROZEN`, or `INSUFFICIENT_FUNDS`, as applicable. Owner/Support attempts are forbidden. The portal rejects browser-supplied replacement account/currency fields for reversals.

This is a wallet credit correction; it does not transfer money back through a bank/MFS provider. Existing unlinked manual adjustments cannot be inferred to be reversals. Review their history before reversing an old deposit; only explicitly linked reversals count toward the automatic cap.

## Booking payment reporting

For native bookings, `captured_amount` and `refunded_amount` now include both issue and ticket-management operations for the same booking, wallet account and currency. Reissue fare differences and debit-direction void fees are included. Released holds are not captures; pending holds are not captures. Separate additive `held_amount` and `released_amount` fields contain exact minor-unit text.

`payment_state` becomes `partially-refunded` or `refunded` when appropriate. The existing `payable` field retains the original accepted quote. These report changes do not mutate ledger entries, ticket evidence or entitlements. Imported/manual ticket-management support remains outside this change.

## Upgrade and rollout

Apply migration 0059 before starting the updated Rust service, then deploy the matching portal changes. Do not run the updated Rust service against schema 0058. Existing readiness checks reject mismatched migrations.

The migration preserves all historical requests and balances. For duplicate historical payments, it seeds one unique claim per identity, leaves every historical credit intact, and blocks further approvals against that payment. It does not silently choose or reverse an erroneous historical credit. The new report aggregation and reversal lookup have supporting indexes.

Read-only historical duplicate review query:

```sql
SELECT wallet_deposit_payment_identity(details) AS payment_identity,
       count(*) AS approved_requests,
       array_agg(public_ref ORDER BY requested_at, id) AS deposit_references
FROM wallet_requests
WHERE kind = 'deposit' AND status = 'approved'
  AND wallet_deposit_payment_identity(details) IS NOT NULL
GROUP BY wallet_deposit_payment_identity(details)
HAVING count(*) > 1;
```

Focused tests: `tests/wallet.rs` and `tests/support/wallet_deposit_controls.rs`, `tests/wallet_upgrade.rs`, and `src/wallet/ticket_management/tests.rs`. All use disposable local databases and synthetic/mock data. Portal checks are `verify-rust-wallet-controls.mjs` and `verify-wallet-deposit-loading.mjs`, alongside existing Rust wallet and ticket-management adapter checks.

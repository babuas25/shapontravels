# Fresh wallet backend checkpoint — 16 September 2026

## Implemented in source

- Fresh PostgreSQL financial accounts and immutable ledger; exact integer money, owner/currency isolation, atomic reservation/capture/release and cumulative refund limits.
- Client Balance/Statement endpoints with `wallet:read`, no account creation on public reads, fixed statement snapshot and safe field projection.
- Private typed portal service: settings, all five deposit methods, trusted configuration/fee snapshots, independent reviewer approval, adjustments, freeze, paginated operational lists and full report aggregates.
- Durable notification jobs and claim/complete/retry bookkeeping. Actual email/SMS delivery integration is not included in this checkpoint.
- Native held NewTicket reserves accepted payable before supplier dispatch. Successful ticket evidence and financial settlement have recoverable persistence boundaries; saved verification and reconciliation share a capture finalizer. Normal issue performs no PNR lookup.
- Initial Next real-role/agency bridge and safe numeric UI converters. The existing wallet routes and screens have not been switched.

## Checks completed

| Check | Result |
| --- | --- |
| `cargo test --locked` | 73 ordinary tests passed; explicitly gated database/live/private-evidence tests remained ignored in this command |
| Wallet PostgreSQL suite, empty `codex_fresh_wallet_test` | Passed; migrations 29–31 included |
| Full PostgreSQL regression suite, empty `codex_full_wallet_test` | Passed, including native ticket capture failure and recovery |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `cargo fmt --all -- --check` | Passed |
| Frontend `npm run typecheck` | Passed |
| Frontend `node scripts/verify-rust-wallet.mjs` | Passed; no network or financial writes |
| Frontend `npm run verify:api-management` | Passed |
| ESLint for the changed wallet bridge, API permission and verification files | Passed |
| Both repositories `git diff --check` | Passed |

Database scenarios exercised: fresh zero account, duplicate credit and conflicting payload, concurrent overspend, same reservation across different IDs, immutable balance/ledger invariants, frozen capture versus blocked new spending, release replay, refund cap, scoped client reads, no wallet auto-creation, statement pagination across concurrent writes, type-independent balances, cross-owner access denial, settings roles/version/uniqueness, all deposit channels, fee snapshot stability, concurrent reviews, self-approval denial, rejected deposits without ledger credit, adjustment rollback on frozen debit, notification claim ownership/replay, complete report totals and restricted runtime privileges.

Native ticket scenarios include: unconfigured wallet/insufficient funds prevent supplier dispatch; issue and financial reservation roll back together on failure; concurrent different idempotency keys reserve/capture once; accepted payable is charged; PNR unavailable does not block normal issue; unknown supplier outcomes retain funds; an injected capture failure preserves successful ticket evidence and returns processing; subsequent saved verification captures without another supplier issue call.

## Activation and remaining scope

No portal or production database was migrated for this checkpoint. Tests used disposable local databases and synthetic identities/payment data. No old financial data was imported, no actual supplier booking/issue/cancel was sent, and no real email/SMS was delivered. The existing wallet UI, routes and financial storage helpers were not removed.

Full cutover remains pending: existing UI handlers/forms/assets/report integration, recipient-specific notification delivery/resend, portal issue controls, proof-bound non-issuance release, approved refund/reissue/void settlements, imported/manual consumers, complete browser parity and activation/recovery rehearsal. See [current phase plan](../FRESH_RUST_WALLET_PLAN.md) and [implemented API contract](../WALLET_API.md).

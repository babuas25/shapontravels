# Wallet readiness review — 18 September 2026

Verdict: do not sign off the full wallet feature set for production yet. Core local deposit and ledger behavior is working, but settlement coverage, operational recovery and deployment validation remain incomplete.

Scope: current Rust and Next workspace source, current localhost configuration, read-only local financial consistency checks, and fresh isolated integration tests. This is not an audit of a deployed production server, external bank reconciliation, or a guarantee that all failure modes are covered. Older staged documentation was checked against current code and configuration where possible.

## Verified in this review

- Current localhost schema includes migration 49 (wallet authority lock ordering).
- Read-only checks across all 3 current wallet accounts found zero latest-ledger/version/balance mismatches, zero reserved-hold mismatches and zero approved-request/credit mismatches. No native ticket issue rows exist in this local dataset, so these reads do not demonstrate live ticket settlement.
- `cargo test --locked --test wallet --test wallet_notifications -- --ignored` passed both opt-in suites using two newly created disposable databases. The running portal database was not used for fixtures. The suites cover core balance/ledger/workflow/permission invariants and notification claims/retries/financial isolation.
- Frontend checks passed: `verify-rust-wallet.mjs`, `verify-wallet-deposit-loading.mjs`, `verify-wallet-notification-providers.mjs`, `verify-rust-identity-business.mjs`.
- Provider tests used synthetic/in-memory transports. No actual email/SMS, supplier Book/Issue/Cancel, deposit approval or financial posting was performed by this review.

## Findings and release requirements

1. **Refund/post-ticket coverage is incomplete.** `shopontravels/app/api/wallet/refunds/route.ts:23` routes Rust wallet refund requests to `rustWalletHandler("unavailable")`. The financial kernel having refund support does not make the actual portal refund workflow available. The wallet phase plan also retains open refund/void/reissue/imported/manual settlement integration. Complete and test each enabled workflow against the original charge and remaining entitlement, or explicitly exclude unsupported features from the release scope and UI.
2. **Wallet notifications are not operational in the current local setup.** `SHAPON_WALLET_NOTIFICATIONS_ENABLED` is not true; scheduler secret, identity mail token and designated admin SMS recipients are absent. There are 4 pending wallet notification events and no recipient-delivery rows. Configure and test the authenticated scheduler, recipient preparation, approved controlled delivery and queue/failure alerting before promising notification functionality. Do not simply enable delivery against the existing backlog without reviewing recipients and events.
3. **Stuck/pending ticket-worker recovery remains incomplete.** `src/wallet/nonissuance.rs:138` correctly blocks release for a pending worker with `TICKET_WORKER_UNRESOLVED`. Completed unknown outcomes have a separate evidence/reviewer flow, but the phase plan still leaves crashed/pending-worker no-dispatch proof open. Preserve the fail-closed hold; add a proven recovery path and crash-boundary tests, never a timeout-based fund release.
4. **Financial UI error states still need work.** `shopontravels/components/dashboard/wallet/FinancialWalletManager.tsx:896` initializes balances/adjustments/activity as empty arrays; failed requests retain these or older values and report only a shared error message. Totals at line 941 reduce empty wallets to zero. Deposit-specific loading/error gating now exists, but other financial sections can still display zero/empty/stale data alongside a load error. Add explicit per-section loading/error/stale presentation and test it before treating those displays as reliable financial summaries.
5. **Production operational sign-off is not established.** The inspected app uses canonical Rust authority locally with Clerk test keys. Local backup/import checks and prior local restore/cutover evidence do not establish production deployment readiness. Verify the actual production identity/HTTPS configuration, restricted database runtime role, matched migrations/runtime, scheduler, monitoring, representative-volume behavior, and production backup/restore/recovery procedures. No production environment was accessed in this review.

## Evidence

Private test logs and disposable database names: `.local/wallet-readiness-20260918/`. Existing related evidence: `WALLET_LOCK_ORDER_2026-09-18.md`, `RUST_IDENTITY_LOCAL_CUTOVER_2026-09-17.md`, and `LOCAL_PAYMENT_ACCOUNTS_IMPORT_2026-09-18.md`. No live configuration was changed by this assessment.

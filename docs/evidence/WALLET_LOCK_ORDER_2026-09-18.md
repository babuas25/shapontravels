# Wallet reservation/capture lock order — 18 September 2026

Two bookings sharing one remaining wallet balance exposed an inversion between
the identity authority advisory lock and wallet row locks. Reservation held the
authority barrier while waiting for the wallet owner. The saved-ticket finalizer
held the owner/account and requested the barrier from a ledger write trigger.
PostgreSQL aborted one request with a deadlock, producing HTTP 503 instead of the
expected insufficient-funds refusal.

## Change

- Legacy/API business transactions acquire the existing identity barrier before
  domain rows, matching canonical transactions.
- Ticket outcome persistence and saved-proof finalization acquire the barrier
  before ticket rows. They do not reauthorize a completed supplier operation.
- Wallet account locking acquires the barrier before owner/account rows.
- Migration `0049_wallet_authority_lock_order.sql` replaces the posting function
  with the same financial rules and grants, acquiring the barrier first. No
  historical migration, balance, ledger entry, amount or idempotency rule changes.
- Supplier dispatch remains outside database transactions. A barrier timeout
  after dispatch remains an unknown outcome; it cannot authorize a new dispatch.

## Verification

- Native disposable database integration passes, including deterministic lock
  tests using `pg_locks` and `NOWAIT`: waiting finalizers cannot hold ticket/wallet
  rows, and both Rust account locking and SQL posting wait before wallet rows.
- Full Next/Rust portal integration now passes the formerly failing race:
  one HTTP 200, one HTTP 409 `INSUFFICIENT_FUNDS`, one supplier issue and one
  reserve/capture. The prior unresolved hold stays intact.
- The same portal suite passes lost acknowledgements, injected capture failure,
  saved-proof recovery, unknown outcomes, concurrent replay and reviewed release.
- The dedicated wallet suite passes overspend protection, refund limits, frozen
  wallet behavior, immutable ledger, exact balances and restricted runtime grants.
- Canonical identity/business integration passes.
- Migration upgrade on a copy of retained synthetic migration-0048 data, followed
  by a repeat migration, preserves exact snapshots of wallet owners/accounts,
  operations, ledger, bookings, tickets, accepted pricing and function grants.

All supplier/provider responses and financial operations used for these tests
are synthetic. No real Book, Issue, Cancel or wallet posting was sent.

## Deployment

Deploy the matching binary and migration together through the normal backup,
migration, grants and readiness workflow. The global barrier already serializes
business writes through triggers; this change acquires it earlier to remove the
inversion. Its existing two-second lock timeout remains in place.

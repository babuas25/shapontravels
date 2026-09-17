# Fresh wallet portal adapter — verified checkpoint

Date: 16 September 2026. This follows the [backend checkpoint](FRESH_WALLET_BACKEND_2026-09-16.md). It is a staged portal integration, not a completed migration or production activation.

## Change delivered

The existing Next wallet route family and server-rendered owner wallet loader now select the Rust service under the server-only `SHAPON_WALLET_BACKEND=rust-preview` setting. Balance/activity, deposits, reviews, adjustments, freeze, reports, bank/MFS settings/assets and owner saved bank accounts keep the existing UI and response envelopes. Deposit and adjustment forms retain stable submission UUIDs; configuration edits pass optimistic versions.

The trusted bridge resolves the actual Clerk role and persisted agency owner. It validates same-origin mutations, uploads private receipt proof, computes a SHA-256 digest over its bytes, and signs links only after authorized reads. Rust binds deposit idempotency to the payload and digest rather than a newly generated upload handle. Lost responses are retriable without creating a second request/credit; duplicate unreferenced upload objects are discarded after a confirmed replay. Ambiguous uploads remain retained for later reconciliation.

The private Rust service adds scoped request lookup, owner-scoped portal ledger reads, per-currency frozen counts and the recorded portal booking creator in payment reports. Public machine Statement redaction and snapshot pagination remain separate. Normal ticketing still has no new PNR preflight.

## Executed verification

| Check | Result |
| --- | --- |
| `cargo test --locked` | 73 ordinary tests passed; opt-in live/private/DB tests remained gated |
| Wallet PostgreSQL invariant suite | Passed on new `adapter_final_wallet_test`; includes changed upload handles/same proof replay, changed digest conflict, foreign/wrong-kind request lookup, complete aggregates, runtime privileges and previous financial race/invariant checks |
| Actual Next route modules → authenticated Rust HTTP → PostgreSQL | Passed on new `api_portal_local_wallet_adapter_03`; previous adapter run also passed on `_02` |
| Actual owner server component branch | Returned Rust payment options/sender accounts under preview without calling the legacy session/financial loaders |
| TypeScript typecheck | Passed |
| `verify:rust-wallet` | Passed exact units, unsafe integer rejection, booking report contract, canonical ownership, actual role and provisioning checks |
| `verify:api-management` | Passed |
| Existing `verify:wallet` | Passed static legacy schema/ownership/issue-order checks |
| ESLint, Rust clippy `--all-targets -D warnings`, fmt, both diff whitespace checks | Passed |

### End-to-end assertions

- Fresh owner account starts at zero; shared B2B sub-user resolves to the same account; a foreign agency sees none of its requests/ledger.
- Bank/MFS create replay, changed-payload rejection, versioned bank logo/MFS QR upload, stale version rejection and sender account create/edit/delete.
- Five deposit channels with immutable bank/sender/MFS snapshots. Synthetic BDT 100 at a 1.25% fee credits exactly 9,875 minor units.
- A simulated lost HTTP reply **after** Rust commits a deposit is retried with the same ID and proof. The saved attachment is reused. Concurrent retries return one request.
- Actual review route approves once; repeated identical review does not credit twice; a wrong-kind review route returns not found.
- An adjustment requester cannot approve their own request; another finance operator can. Freeze blocks discretionary debit; unfreeze permits it.
- Report/ledger totals agree with the resulting three synthetic postings. Quote major units and ledger minor units map correctly to existing report fields.
- Inactive accounts, disabled B2C, cross-origin writes and forged preview-role cookies fail.
- Preview refund and decision-email resend fail explicitly. Old wallet table access and old RPC execution fail before reaching the legacy store.
- Notification rows remain pending; no actual provider delivery was attempted.

## Isolation and limits

The test runner uses synthetic Clerk/directory/asset providers, rejects all external fetches and starts/stops its own Rust process on loopback port 18083. It requires an empty disposable database. Tests created no supplier booking, ticket, cancellation, email or SMS. No old financial data was copied. `.env.local`, the existing portal database and the running portal backend were not switched or reset. No commit/push/deploy was performed.

`rust-preview` deliberately disables all legacy Supabase RPCs and legacy financial tables. This broad isolation is suitable for the staged test portal, not a production feature flag. Full refund/manual/ticket-management consumers, notification delivery/resend, snapshot-aware asset cleanup, browser parity, large-account behavior and activation/rollback work remain open in the phase plan. HTTP/component tests do not establish complete browser interaction parity.

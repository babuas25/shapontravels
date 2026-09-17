# Identity authority dependency inventory

Reviewed 2026-09-17 against the uncommitted worktrees in both repositories. This inventory describes `staged` Rust plus local `rust-preview` Next. Default/live authority remains legacy. It is not a claim that all Supabase features have been migrated.

| Surface | Canonical authority and data | Remaining transport/dependency |
| --- | --- | --- |
| Clerk login, credentials, sessions, MFA/recovery | Clerk authenticates; Rust determines application role/status | Existing Clerk sign-in/up and UserProfile widget retained |
| Dashboard account/session | Rust session, user and current agency membership | Next verifies actual Clerk session; preview cookies never grant access |
| Users & Roles / Sub Users | Rust roster, lifecycle commands, agency versions, scoped membership | Clerk avatar/last-sign-in hydration uses only Rust-authorized IDs; metadata is ignored. Recent-sign-in sorting scans at most 1000 matched canonical records and fails explicitly beyond that bound |
| Create/invite/remove/reactivate | Rust durable intent, authority transaction, provider attempt/fence, outbox, recovery | Existing Clerk adapter; environment-enabled writes limited to disposable test mode |
| Profiles/company/staff/B2B approval | Rust profiles, applications, versions, documents, branding, names | Authenticated Cloudinary asset transport, deliberately test-only in preview |
| Wallet summary/deposit/review/statement/receipts | Existing Rust wallet with canonical agency owner and actor | Synthetic financial verification; no imported balances. Attachment transport remains Cloudinary with the preview test-cloud gate |
| Finance receivers / agency/actor labels | Canonical Rust directory and scoped lookup | No Clerk role scan or Supabase agency/profile read in this branch |
| Saved passengers | Rust profile storage and canonical actor; draft owner rechecked for scoped saving | Existing validation/presentation retained; customer business access denied |
| Search/reprice/price acceptance | Separate five-minute `sti_` sessions bound to user authorization version and client | Supplier adapters unchanged; tests use fake suppliers. No Book/Issue scope from a search session |
| Holds / assignees / booking receipts | Rust current actor, active target owner, client and agency wallet binding | Existing native hold workflow: Super Admin on behalf / B2B owner submission. Admin/Support/Accounts can read; agency sub-user reads are owner scoped. Supplier cost is Super Admin only |
| API clients / tier policy / credentials | Rust actor and target checks, management version checks, linked client revocation | New B2B account does not automatically enable external API access; explicit client provisioning remains required |
| Wallet notification expansion | Rust claim-bound requester/reviewer/company directory | `stim_` worker credential distinct from interactive `stib_`; existing recipient delivery leases/fences retained |
| Wallet delivery | Existing one-attempt mail/SMS adapter contracts | Preview email only to loopback SMTP; SMS intentionally denied in preview. No real delivery was tested |
| Authentication emails | Signed Clerk `email.created` payload and original auth template | Existing route retained; canonical branch restricts to loopback test SMTP. Application welcome comes from Rust, preventing legacy duplicate welcome |
| User lifecycle events | Signature-verified Next boundary → Rust durable inbox | Separate webhook/event capability; metadata events do not grant authority |
| Setup and recovery | Authenticated readiness counts, status/setup page, paginated recovery UI | Operator selection and deployment configuration remain external prerequisites |

## Closed legacy paths

`lib/db/{users,agencies,profiles,staff,sub-users,security,upgrade-requests}.ts` reject canonical-preview calls before legacy work. Existing user/agency/profile/application server actions also reject direct invocation in preview; middleware blocks **all** unported Server Actions. Generic Rust administrator transport is guarded. Dedicated business adapters select a closed Rust route allowlist; there is no browser-accessible arbitrary business proxy.

When configured, the Supabase client additionally denies identity tables and all old RPCs in canonical preview. Domain-level guards still fail closed when Supabase configuration is absent. Ported account/profile/application requests do not interpret a failed read as an empty writable form.

The Clerk auth-email relay and wallet scheduler retain their own signature/bearer authentication rather than interactive session authentication. Their routes are specifically allowlisted; other old workers are blocked in preview.

## Features outside the identity migration

Marketing content, announcements, promotional popups, search-history suggestions, supplier configuration, markup/search controls, manual/imported legacy bookings, legacy booking reconciliation, ticket-management void/refund/reissue, and unrelated media settings still have legacy dependencies. The canonical preview uses the existing dashboard shell with only ported navigation. It does not silently fall back to these old identity/business writers. Native Rust Search → Reprice → Hold checkout routes are allowed; legacy checkout attempts without a Rust hold draft show an access/setup state.

Existing native Rust restrictions remain deliberate: staff/sub-user booking submission is not expanded to legacy booking workflows, a new B2B account does not gain machine API permissions, and funds/history are not transferred by an email/name match. These are not a full migration of the unrelated legacy booking subsystem.

## Activation boundary

Canonical authority selection is implemented behind a durable migration-0046 rollout marker and release pin, but it is not configured or enabled by this delivery. `rust-preview` remains local-only; canonical requests fail closed without a matching marker/header pin. Real target mapping, selected operator bootstrap, actual provider/storage/mail configuration, coordinated writer shutdown and controlled activation remain Phase 9 gates. Keep the current legacy deployment until those gates are explicitly resolved. Do not remove Supabase globally based on this identity inventory.

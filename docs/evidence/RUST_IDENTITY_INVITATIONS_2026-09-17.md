# Phase 4 durable invitation evidence — 2026-09-17

Status: staged invitation issue/revoke/verified-acceptance orchestration verified with synthetic providers. **Phase 4 remains in progress.** Production invitation/create/effect adapters and workers, mail delivery, expiry semantics, deletion, signed durable inbox and recovery UI remain unimplemented or unwired. This is not live portal migration or real invitation delivery.

Existing uncommitted work, Next frontend authority gates, Clerk login, templates/redirects, real accounts/data and environment secrets were preserved. No real provider writes, invitations, email delivery or account deletion occurred. Tests used only a dedicated loopback PostgreSQL cluster and in-memory provider proofs.

## Implemented

- Migration `0039_portal_identity_invitations.sql`: immutable invitation intent, issuer, reserved new user UUID, normalized email/hash and role/agency/version snapshot; separate durable fenced attempts; branded-mail intent; lifecycle/retention guards; ordinary `accept_invitation` operation action. It does not import rows or activate any writer.
- `src/identity/invitations.rs`: strict manager/own-agency commands, fresh authority checks, cross-create email and operation-ID reservations, shared agency capacity, local irreversible revocation, issue/revoke/observation claims, bounded safe retries, unknown-outcome recovery and verified acceptance.
- Staged `/invitations/prepare`, `/dispatch`, `/query`, `/revoke`, `/accept` routes share the existing bridge and HTTP boundaries. The environment-configured runtime has no invitation adapter. Only explicit synthetic injection enables writes/acceptance evidence reads; no browser force-complete endpoint exists.
- Owner input carries only email plus the empty `own_agency` grant; Rust derives B2B-sub role and current active owned agency/version. Managers use existing grant permissions. Same-agency owners or permitted current managers can query/revoke; original issuer authority is rechecked before issue and acceptance.
- Prepare consumes 20/hour; manager revoke consumes 30/hour; owner revoke shares the 40/hour access bucket. Exact prepare/revoke replays do not charge another action. A separate dispatch retry limit caps proven-not-sent issue/revoke calls at five, with 30-second delays.
- Revocation request commits locally before provider revocation: it blocks acceptance and pending mail immediately, including unknown issue/revoke outcomes. Pending provider work remains visibly unresolved until correlated evidence confirms it; revocation is never assumed from a timeout.
- Acceptance requires exact operation/invitation correlation, accepted provider state, the authenticated provider user and provider-verified email. The final transaction rechecks issuer, grant, agency/version/capacity and revocation. Email-only matching or browser metadata cannot grant authority.
- `src/identity/provisioning.rs` extracts the existing create finalization into a shared atomic insert path. Both workflows refuse existing mapped identities and retained financial/business references, grant fresh identity/membership, provision a new zero BDT agency wallet where needed, and save ordinary provider effects. The account-create matrix was rerun against this refactor.
- Automatic onboarding denies a pending invitation's verified email reservation without adopting its identity. Same caveat as creation: uncorrelated/no-verified-email onboarding races remain matching-review conflicts, requiring provider/inbox coordination before production activation.
- Mail intent is pending/blocked only. No ticket or bearer URL is persisted or returned, and no sender, delivery confirmation or mail retry worker is implemented. Existing branded transport/template/redirect integration remains a separate gate.

## Verification executed

All database work used the dedicated cluster at `127.0.0.1:55451`. Fresh-install suites require new empty databases; upgrade uses a new clone of synthetic evidence. No existing database was reset. Databases remain retained; the dedicated cluster was stopped after verification.

| Check | Result |
| --- | --- |
| `cargo check --locked` | Passed |
| `cargo test --locked --quiet` | 84 ordinary tests passed; live/browser/database suites remain opt-in |
| Invitation PostgreSQL lifecycle matrix | Passed in `phase4_invitations_04_identity_test` |
| Strict invitation DTO/input test | Passed in ordinary suite |
| Additive migration upgrade | Passed in `phase4_invite_upgrade_identity_test`, cloned from migration-0038 `phase4_creates_03_identity_test` |
| Upgrade integrity | 17 existing identity/agency/create/attempt/audit/financial/control/rate table fingerprints unchanged; all three new invitation tables empty |
| Foundation regression | Passed in `phase4_invite_foundation_identity_test` |
| Bootstrap/session/onboarding regression | Passed in `phase4_invite_routes_identity_test` |
| Role/access/fenced effect regression | Passed in `phase4_invite_ops_identity_test` |
| Agency lifecycle regression | Passed in `phase4_invite_agencies_identity_test` |
| Durable create/recovery regression | Passed in `phase4_invite_create_identity_test` |
| `cargo fmt --check`, `git diff --check` | Passed |

Earlier invitation databases `_01`, `_02`, `_03` remain retained. Those runs exposed test-fixture issues: wrong wallet balance column, an assertion that rejected harmless identical unknown-result replay, and agency codes missing their required prefix. The final fresh `_04` run passed the corrected complete matrix. An expired worker's conflicting successful result remains rejected; identical already-recorded unknown replay changes no state.

The invitation matrix verifies:

1. Wrong bridge credential families, unauthorized actors, Admin-to-Super-Admin grant rejection, malformed email, strict caller-redirect rejection, and no owner role/agency override. Provider-banned actors cannot query through HTTP.
2. Audit failure rolls back preparation completely. Concurrent exact preparation creates one intent; changed request conflicts; same normalized email and UUID cannot bypass reservations through either create or invitation workflows. Pending invitation email blocks automatic onboarding without assigning an identity.
3. Durable attempt/audit precedes provider I/O. The fake acquires the same authority lock during the call, proving that no transaction is held over the external boundary. Attempt-audit failure causes zero provider calls; concurrent dispatch issues once.
4. Safe views omit email, provider ID and invitation secrets. Pending provider issuance creates only a mail intent and no application identity.
5. Pending/unaccepted evidence, mismatched subject, missing verified email and wrong operation correlation cannot grant access. Existing canonical users and retained client/business references require matching review rather than adoption/promotion.
6. Acceptance audit failure rolls back the entire identity/agency/wallet/effect grant. Concurrent verified acceptance commits once and returns the same internal identity; replay through the real HTTP route succeeds without creating another account.
7. B2B acceptance creates one fresh BDT wallet at zero available/held balance. Customer remains onboarding. Sub-user acceptance creates only the stored agency membership. Ordinary effects remain queued after local acceptance.
8. Stale revoke versions conflict; revoke-audit failure leaves no partial deny/mail change. Committed local revoke blocks acceptance and mail before any provider response; repeated request preserves the same version.
9. Unknown provider revoke blocks resend and remains locally denied until correlated read confirms revocation. Unknown issue cannot resend or release its email reservation; a revoke request survives issue reconciliation and blocks mail until provider revoke completes.
10. Revocation committed during the acceptance evidence read prevents the final grant. Acceptance of an already revoked invitation never reopens access; revoking an already accepted invitation is rejected.
11. Provider success plus result-audit failure leaves the durable claim, then lease recovery/read reconciliation finds the original issue without a second write.
12. Expired issue/revoke/observation leases record unknown outcomes. Stale fences and conflicting late confirmations are rejected. Identical already-recorded unknown completion is harmless. Read absence or still-pending revoke evidence cannot enable resend or grant access.
13. Proven-not-sent issue retries obey delay and five-attempt cap, then safely cancel. Proven-not-sent revoke reaches unresolved state at the cap and retains the deny flag. Prepared cancellation makes no provider call.
14. Original issuer demotion blocks both pending issue and acceptance. Agency/owner suspension blocks acceptance. Another owner and ordinary sub-user cannot query/revoke the invitation.
15. Agency capacity sums memberships, pending creates and pending invitations; at 100 reserved slots neither workflow can prepare another member. Acceptance consumes its own reserved slot without double counting.
16. Shared rate buckets enforce limits; exact replay bypasses a second charge. Intent/attempt/mail retention and immutable grant/revocation guards reject destructive rewrites. Environment runtime dispatch/acceptance remain disabled.

## Reproduction

Create a **new empty** loopback database ending `_identity_test`, set `IDENTITY_INVITATIONS_TEST_DATABASE_URL`, then run:

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_invitations -- --include-ignored --skip additive_invitation_upgrade_preserves_existing_evidence --nocapture
```

For upgrade, create a **new clone** of synthetic migration-0038 evidence; the name must include `invite_upgrade` and end `_identity_test`. Set `IDENTITY_INVITATION_UPGRADE_TEST_DATABASE_URL` to that clone and run:

```sh
/Users/ashifbabu/.cargo/bin/cargo test --locked --test identity_invitations additive_invitation_upgrade_preserves_existing_evidence -- --ignored --nocapture
```

The previous five identity database suites were explicitly run with separate fresh database URLs and `--ignored --skip additive_`. Their older migration-specific upgrade tests were not rerun here; the new migration-0038 upgrade test exercises this slice's additive boundary.

Never load real provider credentials, send to real people, reset evidence databases or apply these test commands to a live database.

## Remaining gates and next work

- Continue Phase 4 with deletion dependency preview and durable deletion workflow. Preserve all financial/reference evidence and use synthetic provider deletion only.
- Verify official Clerk invitation acceptance/correlation/expiry and error semantics before implementing a real adapter. The synthetic `Snapshot`/`NotSent` contract is not a claim that the provider already exposes equivalent evidence. Missing evidence must fail closed.
- Add signed durable event inbox and read reconciliation coordination, bounded queue discovery, production workers, denied-action audit and operational visibility. No public endpoint should accept an arbitrary provider subject, success flag or acceptance URL as proof.
- Implement branded mail delivery/recovery and expiry handling while preserving existing templates and canonical redirect. Pending mail intent is not sent mail; revoked/accepted invitations must remain blocked from dispatch.
- Add recovery UI and the later frontend/business parity work. Keep current Clerk login and all isolated preview gates; do not enable live authority or remove Supabase identity dependencies based on this partial backend slice.

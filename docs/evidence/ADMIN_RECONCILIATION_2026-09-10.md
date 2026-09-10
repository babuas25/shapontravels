# Admin reconciliation — local verification, 2026-09-10

User approved coding-free Admin reconciliation after the explanation of uncertain booking outcomes. This adds only the focused reconciliation screen to the earlier backend-only scope. It does not authorize production Hold/Issue; Triplover UAT remains the only permitted environment for actual booking validation.

## Implemented

- Same-origin Bengali screen at `/admin/reconciliation`: existing human-admin login, unresolved/resolved queues, detail/status recheck, supplier-confirmed outcome form and decision history with administrator names/times. IDs and optimistic versions are handled by the UI.
- Four protected Admin endpoints for list/detail/recheck/resolve. B2B machine tokens cannot use them; no fabricated Machine identity or bypass of commercial endpoint authentication.
- Required evidence, reason, supplier case/confirmation reference and attestation. Row locking plus current `updatedAt` prevent stale/concurrent decisions. Ordinary pending requests cannot be resolved during their first five minutes.
- Immutable resolution/evidence history, atomic redacted audit, explicit `manually_resolved` outcome for owner-only status and original-key replay. This never fabricates a supplier success/price or creates a new reservation.
- Delayed Book completion preserves its response in immutable late-outcome storage and reopens review without deleting the administrator's earlier decision. A subsequent decision adds history. No duplicate supplier Book is dispatched.
- Failed/mismatched PNR evidence is stored but not presented as verified supplier status. The normal original-supplier servicing control remains enforced.
- Public HTML contains no private booking data. Same-origin CSP, textContent rendering, memory-only session tokens, no third-party browser resources, uploads or URL fetching.

## Verification

- Final database integration: `migrations_and_constraints` passed in **37.00 seconds**, with disposable PostgreSQL only and synthetic passengers/mocked suppliers.
- Covers machine/admin separation, all four outcomes, missing attestation, stale versions, concurrent resolution, immutable history/audit, owner-only manual status, no mutation during administrative decisions/rechecks, pending-age restriction, delayed worker reopening, second-decision history and unchanged idempotency replay behavior.
- Validates failed supplier evidence is not shown as verified status, and history includes two retained decisions and administrator names after a late outcome.
- Ordinary suite: 46 tests (34 unit, 6 foundation, 6 fixture); separate ignored live supplier tests remain disabled. Foundation checks include served Admin shell/JS, CSP, authentication protection and unique OpenAPI operation IDs.
- Rust formatting, strict all-target Clippy, JavaScript syntax and diff whitespace checks pass. No browser interaction/visual QA was requested or performed.

## Rollout and remaining limits

Migration `0013_manual_reconciliation.sql` is required with this build. It adds the verification flag, manual booking state and immutable decision/late-response tables. Existing migration and grant workflow includes its tables/sequences. Applied only in disposable integration databases; no production or working database mutation, account provisioning, supplier network call, commit/push or deployment occurred.

Manual `held` records the administrator's verified finding; it does not fill missing supplier references, revalidate accepted prices or unlock ticketing/Cancel. Private evidence access/retention and full UAT end-to-end acceptance remain production-readiness work. Cancel, NewTicket, direct issue and reports remain separate unfinished steps. The screen is served by the existing Rust application; it is not a separately deployed dashboard service.

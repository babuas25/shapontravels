# Imported ticket servicing parity — 20 September 2026

Implemented the user's instruction that imported bookings behave like native Rust API bookings for Refund, Reissue and VOID. See [implementation contract](../IMPORTED_TICKET_MANAGEMENT.md).

Verified:

- Full ordinary Rust test suite and strict all-target Clippy pass.
- Dedicated database journeys cover all three import sources, partial refunds, concurrent duplicate settlement, reissue debit/capture and subsequent replacement-ticket refund, VOID deadline and credit, API client scoping/replay, receipt projections and unchanged supplier evidence.
- Disposable wallet-kernel and dashboard-import database suites pass, including runtime database-role permissions, refund caps and import replay.
- Frontend TypeScript, targeted ESLint, native route checks and actual import-page prop checks pass.
- Existing localhost data was backed up and restored into a separate database. Migration 0060 was rehearsed on that restore, then applied locally. Every pre-existing business table matched its pre-migration fingerprint. The new financial-subject table was populated without rewriting existing ledger or booking rows.
- The pre-migration backup contains an existing unqualified SQL function used in the imported booking reference generated column. Restore required setting the dump session's search_path to public when replaying into the isolated empty database. All restored tables and sequences matched before rehearsal. This pre-existing restore limitation was not changed by this feature.
- Local rollout revision 15 is active; backend readiness returns HTTP 200.
- The signed-in browser renders Refund and Reissue on booking STR0A6ABI0A6ABI. Refund opens successfully. Availability and list requests return HTTP 200. The saved entitlement equals the actual captured BDT 4,234.00, not the ticket's gross face value. VOID is correctly absent because this ticket's issue-day window is closed.
- The current browser is signed in as Super Admin, so the panel correctly requires the owner to initiate a request. B2B owner action props and the owner-to-staff-to-finance workflow were verified in isolated tests.
- Local request count remains zero and the affected captured operation has zero refunded amount. No real refund, supplier mutation or customer notification was submitted. No production deployment was performed.

Private backup, restore, migration, rollout and read-only verification evidence lives under `.local/import-ticket-management-release/`.

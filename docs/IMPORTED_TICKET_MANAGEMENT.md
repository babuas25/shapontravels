# Imported and native ticket servicing

User direction, 20 September 2026: imported bookings use the same Refund, Reissue and VOID workflow as Rust API bookings. This supersedes the source-based servicing restrictions in the frontend reference project.

Migration 0060 adds a financial booking identity linked to exactly one native or imported record. Existing ticket-management and wallet booking references use that identity. Original booking evidence, wallet operations and ledger entries are not rewritten. Refunds against historical import captures are permitted only when the import explicitly owns that same captured operation; unrelated bookings cannot use it.

All sources use the existing passenger/route selection, staff review, quotation, owner decision, assignment and finance settlement. VOID retains its issue-day cutoff at 23:30 Bangladesh time. Supplier work remains manual, as in the native ticket-management workflow.

Modern imports use their accepted Rust per-passenger pricing. Older imports use the actual captured payable, allocated in proportion to saved passenger fare groups with largest-remainder cent rounding and passenger-order tie breaking. A single passenger receives exactly the captured amount. Missing or inconsistent multi-passenger fare evidence is rejected. Reissue fees do not become refundable; the per-passenger fare difference follows the replacement ticket's original funding slices.

Imported receipts now show current ticket numbers, request references and settlement payment state. Booking payment reports include original import capture and all subsequent ticket-management reservations, releases, captures and refunds. Existing imported supplier evidence remains unchanged.

The frontend preserves the shared reference UI and enables its actions for confirmed, wallet-backed imports. Native direct and held tickets use the same Rust action policy. The legacy backend's helper is unchanged.

Verification includes synthetic database journeys for IMP/EXP, manual and supplier imports; partial refund, duplicate settlement, reissue and subsequent refund; VOID time eligibility; scoped API availability/create/replay/detail/list; original evidence preservation; exact allocation; actual frontend route/page wiring; wallet-kernel and dashboard-import regressions; full ordinary Rust tests, strict Clippy, TypeScript and targeted ESLint. No real refund or supplier operation is submitted by this verification.

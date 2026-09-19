# Imported booking visibility and shared ticket document

## Change

Imported bookings now join native bookings before My Bookings filtering, sorting, counting and pagination. Canonical Super Admin reads all imports; B2B partners and sub-users read imports belonging to their current agency, including imports assigned to an agency sub-user. Import management remains Super Admin only.

The new canonical `/admin/portal-imports/receipt` returns `booking` in the frontend's `PublicBooking` contract and `travellers`, with explicit field selection. It omits raw supplier evidence, supplier prices, quote/operation identifiers and internal notes. Import success/replay/completion responses include the same document and the common booking-detail URL. Legacy import-detail links redirect to that URL.

The Next route uses the existing `BookingDetails` component, including the electronic ticket/booking document and print controls. Existing imports require no data migration or repeat wallet capture. Unreconciled supplier fare breakdowns are omitted instead of presenting them as customer charges; the saved customer payable total remains authoritative.

## Verification

- Extended `tests/dashboard_sections.rs` passed against a new, empty `codex_import_booking_20260918b_dashboard_test` database. Coverage includes existing imports, confirmed/on-hold documents, agency and sub-user access, cross-agency denial, internal-data exclusion, status/amount/search filters, stable two-page pagination, and no additional wallet capture from reads or import-only records.
- Frontend TypeScript, targeted ESLint, canonical identity authority verification, Rust build and Clippy passed.
- Local browser: the user's existing import appears alongside six native bookings; opening it displays the shared electronic ticket document, paid status, PNR, ticket number, itinerary and print/download control.
- Browser session remained Super Admin. B2B behavior was tested through canonical Rust requests, not by changing browser identity.
- Read-only verification through the retained local backend using the actual assigned B2B principal passed: the reported import is listed, its confirmed receipt returns the saved BDT 4,234 payable and paid status, its issue time is present, and supplier cost/reference are hidden. Another active agency receives 404 for its receipt. Existing IMP/EXP detail links were also verified to redirect to the shared ticket page.

No import was repeated, no supplier transaction was dispatched, and no booking or wallet record in the retained local database was modified by this correction. No production deployment.

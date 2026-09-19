# Imported booking references and pricing — local verification

Active repositories: `Projects/shapontravels` (Rust) and `Projects/shopontravels` (Next). The Documents frontend was used only as a reference.

## Changes and observed result

- Migration 0056 applied to the retained localhost identity database after a private pg_dump backup. The existing imported booking resolves as `STR0A6ABI0A6ABI`; its old ST-IMP URL remains an alias. Supplier reference displays `BDF26081026123`, independently of airline PNR `0A6ABI`.
- Actual airline retrieval through the local Super Admin form returned base BDT 3,424 and taxes BDT 1,125, gross BDT 4,549. Entered invoice cost BDT 4,234 produced read-only Rust-calculated B2B payable BDT 4,303.61 using the current markup and agency tier. Preview only: no import authorization or charge was submitted.
- The user explicitly confirmed BDT 4,234 as supplier cost. The root-only audited legacy cost correction recorded 423400 minor units. The retained booking still shows captured payable BDT 4,234; My Bookings now shows supplier payable BDT 4,234 and actual realized profit BDT 0 instead of the incorrect negative BDT 315.
- Before/after database snapshots verified unchanged available balance, held balance, wallet version, capture amount/state, booking payable and ledger entry count. The BDT 69.61 difference from today's quote was not debited or booked as earned profit.
- Browser verified canonical reference, actual supplier reference, corrected cost, and calculated preview on localhost:3000.

## Verification

- Rust dashboard integration suite passed on new disposable `codex_import_pricing_20260918b_dashboard_test`: native-format import references, complete public booking shape, agency visibility and isolation, exact once-only capture, price tamper rejection, mismatched authorization, base/tax receipt mapping, and rejection of missing fare evidence or legacy correction against a new priced import.
- Cargo build (main server and local identity example), targeted Clippy with warnings denied, frontend TypeScript and targeted ESLint passed.
- `verify-rust-airline-import-pricing.mjs` passed: real amount fixture, missing/inconsistent fare denial, passenger-type coverage, grouped amount precision, and multi-passenger NDC normalization.
- `verify-impexp-trip-scope.mjs` passed.

## Limits

No production deployment or new real-ticket financial transaction occurred. Existing captured imports retain their original payable. A mixed passenger-type invoice cost cannot be allocated without its fare breakdown; the workflow refuses pricing rather than inventing amounts. Incomplete source evidence must be resolved before import/charge.

## Ticket gross fare follow-up

The imported receipt now exposes an explicit `ticketFare` document for display and printing. It uses the stored gross total and reconciled airline/supplier base, taxes, fees and AIT components, independently of the financial payable. Existing payable fields and ledger records remain unchanged.

Verified the retained ticket in Chrome: Adult x1, base 3,424, taxes 1,125, AIT 0, row amount and total BDT 4,549. The individual passenger print rows also contain BDT 4,549. Super Admin and owning B2B receipt API checks both passed, retaining financial payable BDT 4,234.

Three Rust receipt tests passed (legacy differing payable, grouped supplier gross with AIT/fees, invalid evidence rejection). Dashboard integration passed in new disposable `codex_import_ticket_gross_20260918_dashboard_test`, including distinct gross ticket and wallet payable assertions. Frontend TypeScript, targeted ESLint, local backend build and both repositories' diff whitespace checks passed. No database migration or wallet adjustment was needed for this read/display fix.

## Imported itinerary details follow-up

- Corrected normalization to preserve source departure/arrival terminals, match checked/cabin baggage by route, resolve airport names from the existing airport catalogue, and format minute durations (50 becomes 50m; 250 becomes 4h 10m). The TTInteractive reader also retains arrival terminal evidence.
- Migration 0057 applied locally after a private database backup. It stores immutable itinerary detail snapshots. Super Admin can refresh source details through the ticket page; Rust rejects changed flight identities or schedules, and receipt access remains agency-scoped. Confirmed booking and wallet records are not rewritten.
- Used the new button to retrieve this existing US-Bangla ticket. Browser shows Osmany International Airport, Dhaka / Hazrat Shahjalal International Airport, source departure terminal T-D, duration 50m and source Check-in 20 Kg. The source returned no arrival terminal or cabin allowance.
- User explicitly confirmed Check-in 20 Kg and Cabin 7 Kg. Added the cabin allowance to this ticket only through the audited itinerary command. Browser now shows both allowances. No global or carrier-wide baggage default was introduced.
- Verified financial snapshot unchanged across the baggage update (gross, payable, captured amount, wallet available/held/version, ledger count). Gross ticket total remains BDT 4,549.
- Rust itinerary unit test and dashboard integration suite passed on a new disposable `codex_itinerary_details_20260918_dashboard_test`. Coverage includes Super Admin-only mutation, owning B2B read, flight-change denial, and unchanged payment operations/payable. Frontend TypeScript/ESLint, source normalization/trip-scope scripts, targeted Rust Clippy and build passed.

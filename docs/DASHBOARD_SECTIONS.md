# Rust-backed dashboard sections

The active frontend is `Projects/shopontravels`. `Documents/shapontravels-frontend` is a read-only layout/workflow reference. No reference Supabase tables, RPCs, credentials, or data are adopted by the new routes.

## Access

- Announcements (including detail), Media & Banners, Appearance, and IMP/EXP: Super Admin.
- Reports: Super Admin can choose an agency and view finance; B2B partners can read and export their own agency's sales. Sub-users and staff are excluded from the new report surface.
- Canonical identity is resolved at each bridge request. Rust checks role and current membership again inside its authority transaction. Target users are verified with the identity provider before import operations.
- Next's explicit request allowlist exposes only the new routes. Legacy Server Actions remain blocked.

## Content

Migration 0054 installs versioned content and uploaded-asset metadata. `/admin/site-content` is a canonical Super Admin command endpoint. Saves use compare-and-swap versions and commit their audit record atomically. `/api/site-content` contains only active published messages/offers/slides and current branding, with no editor identity or inactive drafts.

The existing Cloudinary upload service validates signatures, byte size, pixel dimensions and static images, normalizes to WebP, and uses new asset IDs. Rust publishes the metadata with the expected version; old assets are retained rather than destructively overwritten. A concurrent publish cannot silently replace newer content. Optional public branding falls back to an empty presentation on outage; editors do not pretend a failed read succeeded.

## Sales

`/admin/sales-report` reads native verified ticket issues and confirmed imports. It never estimates totals from just a displayed page. The same scoped query supplies pagination, filters, currency totals, user and airline options, and export pages. Exports freeze their upper observation time, use deterministic ordering, and stop with an error rather than silently truncating above 100,000 rows. Monetary SQL aggregates are exact numeric values. B2B payloads exclude supplier cost, raw evidence, passenger documents and wallet internals.

The reference Excel/PDF renderers and report components are retained. The existing finance component continues to use the Rust wallet and includes imported booking payment records.

## Imports

Migration 0055 stores native import quotes, bookings and request replay records. `/admin/portal-imports` handles history, detail, price, read-only supplier retrieval, quote preparation, charge authorization, import and completion.

- Supplier API retrieves report/PNR using the Rust supplier adapter and its configured account. The server-side TypeScript adapter verifies/normalizes supplier evidence; Rust resolves current markup and the assigned agency's B2B tier. The preview's stored quote must match the refreshed evidence and pricing.
- Airline scraping uses the existing server-side airline readers with official-host URL validation. Airline base/tax/AIT components must reconcile to gross; the actual supplier invoice cost is entered separately. Rust resolves current markup and agency tier for both airline and Supplier API imports. The calculated payable is read-only and bound to the reviewed quote before authorization and capture. Missing or inconsistent fare evidence prevents charging; mixed passenger-type invoice costs require an explicit supplier fare breakdown rather than a guessed allocation. Multi-passenger NDC per-person components are normalized to aggregate fare rows.
- Manual import uses the reference structured validation/forms. Confirmed manual records require one distinct ticket number per passenger and an issue timestamp.
- Ownership and monetary snapshots cannot be changed by re-import. Provider/reference uniqueness prevents a second booking. Request replay returns the original result. Quote freshness, target authority and matching authorization are checked before mutation.
- Import Only makes no ledger movement. Confirmed import/completion uses the existing `manual_issue` reservation and capture within the same transaction as the booking and audit. Insufficient funds roll back all writes. Concurrent retries create one booking/capture.
- Imported bookings appear in the same filtered, sorted and paginated My Bookings list as native bookings for Super Admin and their assigned agency's B2B partner/sub-users. Existing imports are included directly; no re-import, duplicate booking or additional charge is needed.
- `/admin/portal-imports/receipt` returns the shared `PublicBooking` document plus travellers. Rust checks current agency membership and explicitly selects public fields, excluding raw supplier evidence, cost and wallet internals. The import success response includes the same booking/traveller document and a `/dashboard/bookings/import/<reference>` URL. Old Super Admin IMP/EXP links redirect there. The frontend uses the existing `BookingDetails` ticket/booking layout and print controls. Its optional `ticketFare` document contains the stored gross total and reconciled base/tax/AIT rows for the ticket and individual passenger copies. This is separate from `totalPrice` and selling fare rows, which retain the booked agency payable for financial consumers. Legacy imports use their stored airline fare evidence; Supplier API imports use explicit aggregate supplier fare components. Missing, non-cent-exact, passenger-count-mismatched or unreconciled components remain unavailable rather than inventing a breakdown.
- Import administration and completion remain Super Admin only. On-hold imports are completed after ticket issuance at the airline/supplier, with a fresh supplier read (or entered evidence for manual records). This workflow does **not** dispatch new supplier ticket issuance or refund/cancellation operations. Existing native booking ticketing is unchanged.
- Imported records appear in the assigned agency's sales report once confirmed. Historic reference data is not copied automatically.

### Import references and legacy cost correction

Migration 0056 derives imported booking references with the same `booking_reference_from_evidence` function as native bookings (`STR` + booking PNR + airline PNR). New imports require unambiguous reference evidence before wallet mutation. Existing `ST-IMP-…` links remain accepted aliases; My Bookings, receipts, import history, sales and wallet views use the canonical reference. Airline lookup PNR remains separate from the entered supplier/old-system reference used for display.

Super Admin `correct_legacy_cost` accepts `{reference,supplier_minor,reason}` for confirmed airline imports created before Rust pricing. It adds one immutable, audited supplier-invoice correction, reflected in booking administration and My Bookings supplier payable/profit. It cannot change captured payable or debit the wallet, and cannot replace the cost of a new Rust-priced import. Historical pricing differences need a separately authorized financial adjustment; they are never silently charged by this migration or correction.

### Itinerary display refresh

Migration 0057 adds immutable source-detail snapshots for imported itineraries. Super Admin `refresh_itinerary` supplements airport names, terminals, route-specific checked/cabin baggage, duration, aircraft and cabin details. Rust checks every segment's airports, airline, flight number and departure/arrival timestamps against the stored itinerary; changed flights/schedules are rejected. Receipts read the latest snapshot through their existing agency scope. The original issued booking, ownership, fares, payable and wallet records remain unchanged.

The frontend provides a Super Admin **Refresh itinerary details** action for airline and Supplier API imports. It retrieves the configured source server-side and submits only itinerary details. New airline imports retain the same details during normalization. Airport names use the existing server-side airport catalogue when absent from the source; terminal/baggage details are never inferred from the route. Numeric minute durations are formatted as hours/minutes. Check-in and cabin allowances are matched to their flight route; conflicting route evidence is not assigned arbitrarily.

### API client retrieval

Airline, Supplier API and Manual imports all share this path. Migration 0058 creates immutable platform transaction/item/price/ticket UUIDs for every existing and future import. Existing machine booking, ticket, report, PNR and booking-pricing endpoints read imports under canonical agency ownership and their normal permissions. The API projection uses native client schemas and excludes supplier invoice/rule internals. See [Client API guide](CLIENT_API_GUIDE.md#imported-bookings-on-the-same-read-endpoints) for saved-evidence semantics and the read-only servicing boundary.

## Verification

`tests/dashboard_sections.rs` runs only against a new, empty loopback database whose name ends in `_dashboard_test`. It covers role denial, cross-agency denial, public draft filtering, conflicting content saves, import retries, one-time wallet capture, insufficient-funds rollback, report filters/totals, finance inclusion, B2B tier pricing and mismatched charge authorizations. Tests use synthetic identities/data and no supplier/Cloudinary/network mutations.

Run with `DASHBOARD_TEST_DATABASE_URL=... cargo test --test dashboard_sections -- --ignored` after explicitly creating that disposable database. Never point it at a retained local or production database.

### Import wallet balance and completion

The import forms show the selected owner's agency wallet **available** balance through Super Admin `GET /api/impexp/wallet`, backed by the Rust import `wallet` command. B2B sub-users resolve to their canonical agency wallet. The response uses exact minor-unit strings and is not cached. Switching owner/currency or refreshing invalidates the prior balance; frozen, missing or unavailable accounts cannot enable a confirmed import.

Airline, Supplier API and Manual confirmed imports require available funds covering User Payable. The forms display a shortfall, disable the charge action when insufficient, refresh after success/failure and on window focus, and hide import/authorization controls after success until the form changes. Confirmed Supplier API imports have no historical no-charge action. Rust rechecks balance before authorization and retains the final atomic wallet reservation/capture, so a concurrent spend cannot bypass the gate. On Hold imports remain available without a wallet debit.

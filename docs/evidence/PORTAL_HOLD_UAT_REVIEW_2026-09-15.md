# Frontend → Rust → Triplover UAT Hold verification — 2026-09-15

Latest continuation: B2B self-booking is now connected and passes isolated Next/Rust/PostgreSQL verification; international UAT searches to SIN and CCU failed at the supplier. See [the continuation record](PORTAL_HOLD_CONTINUATION_2026-09-15.md) for current coverage and outstanding gaps. The findings table below is the original UAT record.

## Design follow-up

The subsequent local design restoration reuses the production Ticket table and booking document. Rust holds now appear inside the original list, and passenger labels, single-name entry and domestic passport handling are restored. The old extra Hold section and local legacy-list error are resolved for Rust Super Admin/B2B sessions. See `shopontravels/docs/BOOKING_DESIGN_RESTORATION.md` for scope, verification and remaining limitations. This follow-up did not create another supplier booking or issue any ticket; the original UAT evidence below remains unchanged.

## Result

**Passed for Super Admin booking on behalf of an active B2B owner.** The actual signed-in Next.js frontend at `http://localhost:3000` called Rust at `http://127.0.0.1:18081`, which used the user's updated Triplover UAT credentials. One real UAT Book request completed as a verified Hold. A subsequent read-only supplier PNR check returned `Booked`.

| Field | Verified value |
|---|---|
| Owner | THE CITY FLYERS |
| Passenger | Arif Hasan — test passenger details |
| Flight | US-Bangla BS-141, DAC → CXB, 29 September 2026, 07:15–08:20 |
| Passenger count | One adult |
| Supplier fare | BDT 4,361.00 |
| Gross fare | BDT 4,861.00 |
| Commission | BDT 275.00 — Basic, existing 55% share of BDT 500 markup |
| Accepted payable | BDT 4,586.00 |
| PNR / airline PNR | 0A4OXY / 0A4OXY |
| Public reference | STR0A4OXY0A4OXY |
| Booking UUID | d207576f-9865-4cfd-8b47-7f7388612acc |
| Hold draft UUID | 40ef5b50-4934-4d29-80ac-e2183a5dd69a |
| Local state / execution mode | `held` / `hold` |
| Supplier PNR status | `Booked`, `item2.isSuccess=true` |
| Supplier deadline | Book: `15/09/2026 12:30:00`; PNR: `09/15/2026 12:30:00`. Supplier timezone remains unspecified. |
| Ticket issue attempts / verified tickets | 0 / 0 |

Receipt: `http://localhost:3000/dashboard/bookings/hold/40ef5b50-4934-4d29-80ac-e2183a5dd69a`.

## Actual browser sequence

1. Search returned 12 flights: nine US-Bangla and three Biman flights.
2. Selected THE CITY FLYERS using the existing on-behalf picker.
3. Selected BS-141 and loaded actual supplier FareRules, including exchange/refund penalties.
4. RePrice produced an owner-scoped holdable fare with the account's existing tier policy. The original supplier, itinerary and gross/commission/payable were preserved.
5. Explicitly accepted the displayed price and entered Arif Hasan's test details in the existing traveller form. No saved customer profile was used for the live supplier request.
6. Reviewed the itinerary, passenger and BDT 4,586 payable, then clicked **Place booking On Hold** once.
7. The normal receipt displayed **Booking On Hold**, PNR, STR reference, deadline and accepted payable.
8. Reloaded the permanent receipt successfully. Booking Management displayed the same record in its Rust on-behalf section.
9. Called the existing Admin `recheck` endpoint once. This made a read-only PNR lookup, returned `Booked`, and saved the latest deadline. It did not change the booking into an issued/ticketed state.

Database evidence confirms one booking, an immutable Super Admin creator association, the selected B2B owner, accepted tier amounts and Hold execution. There are no ticket issue or verification rows. The portal Hold path does not call wallet operations.

## Issues and remaining gaps

Follow-up audit: the legacy-list display problem was addressed by the design restoration described above. The premature Passenger Completed badge is now fixed, along with saved-passenger expiry bounds, impossible calendar dates, leap-day age cutoffs and revalidation after edits on Review. Portal Hold requests enforce the checkout passport expiry bounds from the stored itinerary; domestic documents remain optional. These follow-ups passed the frontend build, dedicated checkout tests, 58 Rust library tests (one private-evidence test ignored), strict Clippy and the local Next → Rust → PostgreSQL Hold suite with simulated supplier responses. The local UAT server was rebuilt and restarted without another supplier booking or ticket issue. See `shopontravels/docs/PASSENGER_VALIDATION_PARITY.md` for current rules and remaining scope. The table below records the original UAT findings.

| Priority | Finding | Evidence / effect |
|---|---|---|
| Medium | Legacy Booking Management list fails locally | The Rust Hold section shows the new record, while the older list below it displays **Bookings could not be loaded**. The page still invokes the legacy booking data path. That list/resume integration needs separate work; this is not a failure of the new Hold record. |
| Medium | Offers above 12 route combinations are omitted | `shopontravels/lib/rust-flights/presentation.ts:46–49` rejects the entire offer above the bound. This was not triggered by the simple UAT route, but remains a code-confirmed coverage gap. |
| Medium | B2B self-booking is not connected | The implemented Hold controller allows Super Admin on-behalf booking. Ordinary B2B search/acceptance still stops before checkout. This UAT pass does not establish B2B self-service Book coverage. |
| Low | Agency metadata is missing for selectable local B2B users | The picker shows agency name unavailable / no Agency ID. THE CITY FLYERS' receipt consequently displays empty separator positions. The owner identity and authorization still persist correctly. |
| Low | Passenger “Completed” badge is premature | `BookingCheckout.tsx:892` only checks surname and date of birth. In browser testing the badge appeared before required passport expiry was entered. Submission validation still rejects incomplete details. |
| Low | Shared checkout wording still says “Confirm” | `CheckoutStepper.tsx:6` labels step three “Confirm”; the review footer says “By confirming”. The final action and receipt correctly say On Hold, and execution remains Hold only. |

No runtime fixes for these UI/integration findings were made as part of this verification.

## Local setup correction and ticketing boundary

At the beginning, PostgreSQL and Rust 18081 were stopped. The existing local review server was configured for FirstTrip/TakeOff, while the current `.env` contains Triplover UAT credentials. Initial FirstTrip/TakeOff startup therefore failed. This was resolved for the requested UAT test by restoring the isolated PostgreSQL cluster and adding an explicit Triplover UAT Hold mode to the existing local server.

`examples/support/portal_hold_uat.rs` accepts only the exact HTTPS hosts `searchapi-uat.triplover.com` and `userapi-uat.triplover.com`, and only when `LOCAL_API_UAT_HOLDS=1`. It reads Triplover settings from the existing Rust `.env`, overrides ticketing to false in memory, and exposes only Search, FareRules, RePrice, PNR and Hold through its transport wrapper. Direct Issue, held-ticket Issue, Cancel and ticket reports remain denied. The wrapper's denial test passes even with an internally ticket-enabled adapter.

The `.env` file itself was preserved. Its `TRIPLOVER_TICKETING_ENABLED=true` does not enable Issue in this verification server. Local database controls also have ticketing disabled for all suppliers. FirstTrip/TakeOff are disabled in this isolated review database so UAT inventory is not mixed with other sources.

The existing local review database was backed up before supplier configuration changes. Its existing migrations were already current through 0027. No remote database configuration, commit, push or deployment was performed. The only supplier write was the requested Triplover UAT Hold.

Restart the verified local server after starting its PostgreSQL cluster:

```sh
LOCAL_API_TEST_DATABASE_URL=postgres://ashifbabu@127.0.0.1:55439/api_portal_local_clerk_20260914 \
LOCAL_API_TEST_RESUME=1 LOCAL_API_TEST_BIND=127.0.0.1:18081 \
LOCAL_API_UAT_HOLDS=1 \
cargo run --locked --example local_api_management
```

## Supporting regression verification

- Rust ordinary suite: 53 unit, six foundation and six production-fixture tests passed. Tests requiring separate explicit external access retained their opt-in gates.
- Full PostgreSQL suite passed in a fresh disposable database, covering portal ownership, latest-price acceptance, expiry, passenger validation, concurrent submissions, recovery, receipts and access restrictions.
- Frontend prebooking and Hold regressions, TypeScript, ESLint and production build passed.
- Formatting and strict Rust Clippy passed; the UAT transport wrapper tests passed.
- Added a repeatable test-only `local_hold_verification` Rust example and `shopontravels/scripts/verify-rust-holds-local.mjs`. They verify real Next handlers → Rust HTTP → PostgreSQL with simulated supplier responses for concurrent submission and timeout recovery; these checks supplement the actual UAT result above.
- Supporting browser checks verified missing-field validation, saved-passenger creation/editing, review, Hold receipt/reload, history and uncertain-outcome display using the existing components. Normal human names are used in the final test harness.

Private raw PNR evidence and database backups are retained under ignored `.local/` paths. This test proves the one-adult, one-way Super Admin-on-behalf path against Triplover UAT. Return/multicity, mixed passenger types and other suppliers were not newly exercised end-to-end against their live test systems in this review.

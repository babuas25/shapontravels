# Portal receipt after an unconfirmed Book response

## Finding

The local Triplover UAT booking request for draft `bea894d5-4b32-41f6-8fc6-32fc62fe0c6d` completed its supplier call at approximately 00:04 Bangladesh time on 16 September 2026. The saved Book envelope contained `item1: null` and `item2.isSuccess: false`. The supplier message described invalid data during an Amadeus Book operation. It did not identify a rejected field. Internal supplier addresses and passenger details are omitted here.

The saved request's Search, item and price references match the accepted RePrice references. Search/RePrice marked the fare bookable, and the request contained the expected one adult. This checks those specific inputs; it does not establish the cause of the supplier validation failure.

No PNR, booking reference or ticketing deadline was returned. Passenger, itinerary and fare data remained saved. The Rust state remains `outcome_unknown`, requiring confirmation before another booking attempt. A supplier business error alone does not prove that no reservation was created upstream.

## Receipt corrections

- The portal projects safe `supplierReportedFailure` and `hasPnrReferences` flags from saved evidence. Raw supplier messages, internal addresses and supplier references are excluded from the browser response.
- The same saved-reference helper constructs PNR lookup payloads and determines whether the receipt has the required lookup references.
- Unconfirmed requests display an awaiting-confirmation header, an explicit supplier-error message, the saved request time and a visible missing-reference placeholder.
- The activity timeline says “Booking request submitted”. The shared receipt no longer invents a default agency license or displays blank contact icons.
- When PNR references are absent, “Reload saved status” only reloads the stored receipt. Held bookings with references retain the explicit status/deadline lookup. Automatic PNR calls remain disabled.

## Verification

- Rust: 72 ordinary tests passed; the full disposable PostgreSQL integration suite passed, including the new business-error response scenario. Formatting and Clippy passed.
- Integration coverage distinguishes a transport timeout from an HTTP-successful supplier error, preserves passenger details, prevents duplicate Book dispatch, blocks PNR dispatch without references and excludes raw supplier errors.
- Frontend: hold adapter/presentation checks, TypeScript, targeted ESLint, deadline notice and isolated print-layout checks passed.
- The existing signed-in local receipt rendered the corrected content. Its saved-status reload completed successfully. Database verification still showed the original three booking records and zero PNR observations for the affected request.
- The local Rust server was rebuilt and restarted; readiness returned success. No live Book, Issue or Cancel was submitted during this investigation.

## Remaining supplier investigation

Triplover must explain the Amadeus validation error and confirm the reservation outcome before a new booking is attempted. No confirmation, PNR or deadline was fabricated, and the failed request was not replayed.

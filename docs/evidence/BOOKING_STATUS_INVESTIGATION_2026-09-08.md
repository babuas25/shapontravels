# Booking status investigation — 2026-09-08

Scope: local source, supplied contract and saved supplier response inspection only. No new supplier calls, portal access, status changes, Book, Issue or Cancel. User reports the current TK booking appears as Hold in the supplier portal; this is user-provided evidence, not independently inspected portal evidence.

## Findings

1. Current TK raw Book response has bookingStatus=Confirmed, isSuccess=true and a nonempty PNR, with no ticketInfoes. SupplierAdapter::book POSTs once and returns bounded_json; bounded_json parses the HTTP body without remapping bookingStatus (src/supplier.rs:174, 267). The diagnostic saves this returned JSON directly. Thus the Confirmed label originates upstream, not in platform status rewriting.
2. The supplied contract describes held Book success as Created (Triploaver_API_Documentation.md:658,758), while its PNR endpoint uses a different status field and gives Booked as an example (line 1135). Its ticketing-report endpoint also uses Confirmed as a filter (line 987). These endpoint labels cannot be treated as interchangeable lifecycle states without supplier semantics. Documentation and observed production Book labels differ.
3. For this TK booking, the combination of raw Confirmed and the user's portal Hold report supports the inference that Confirmed means booking/reservation confirmed while the portal expresses unticketed Hold. This is an inference for this observed case, not proof that every Confirmed response means Hold or a verified supplier internal mapping.
4. Our public held_response guard accepts only bookingStatus=Created (src/booking.rs:394). A response with Confirmed would be classified as outcome_unknown by lines 341–342. This is a local compatibility gap with observed production, distinct from the origin of the supplier label. The diagnostic Book calls used the supplier adapter directly, so no public API state was actually assigned to these bookings.
5. Another local compatibility gap exists at src/booking.rs:424: the guard requires flightInfo-level totalPrice/basePrice/taxes when flightInfo is present. Both captured real Book bodies provide component/passenger prices but omit these top-level fields. Therefore changing only Confirmed acceptance would not make these shapes pass the hold guard. This finding is from static code inspection, not a new public API run.
6. The earlier Ticket in progess response belongs to the previous MH booking, not the current TK booking. No PNR response has been captured for current TK. It cannot establish current TK ticket progress or be used to infer a supplier PNR defect for TK.

## Implications

The current evidence does not establish that the supplier Book status is itself wrong. A mismatch between endpoint terminology, portal lifecycle display and our contract-based interpretation is more strongly supported. Confirmed must not be presented as evidence of ticket issuance by itself; absence of ticket numbers in a Book body alone also cannot prove no ticket has subsequently been issued.

Potential implementation direction, not applied: preserve supplier raw status separately from normalized booking/ticket status; validate documented/observed hold aliases with supplier semantics and ticket/PNR evidence; support Book component-only price representation after verifying counts, totals and accepted Reprice consistency. Do not globally map Confirmed to Held or Ticketed based on this single case. Existing tax coverage issue is separate.

Next evidence that would narrow the cause: user-authorized read-only PNR query for the current TK booking, compared with portal Hold, followed if needed by supplier clarification of Book bookingStatus versus PNR status versus portal status. Each next supplier flow still requires user permission.

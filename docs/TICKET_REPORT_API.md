# Ticket details / report API

Read a supplier report for an owned, verified issued booking. This operation never sends Book, NewTicket, Cancel or another mutation, and it never resolves an uncertain ticket issue automatically.

## Routes

All routes require a machine token with `ticketing` permission:

- `GET /api/bookings/{bookingUuid}/ticket/report`
- `GET /api/bookings/by-reference/{STR-reference}/ticket/report`
- `GET /api/B2BReport/AirTicketingDetails/{platformSearchUuid}/Confirmed`

The first two routes select a particular booking. The supplier-compatible transaction route accepts the platform Search UUID, not a raw supplier transaction. Multiple owned bookings sharing that search return 409; select a booking UUID instead. A duplicate STR reference also returns 409. Unknown or foreign records return 404. Only the `Confirmed` report filter is currently supported.

Use `GET /api/bookings/{id}/ticket` when only the saved ticket receipt is needed. `/ticket/report` reads the original supplier every time; it does not fall back to stale data on failure. `X-Booking-Reference` carries the established public reference when available.

## Verification and response

The supplier GET request uses the stored original transaction reference, encoded as one URL path segment. Supplier servicing must be enabled; Search participation, booking and ticket-issue enablement are not required for this read. The supplier read has a response-size bound, a timeout, one authentication refresh and one transient retry.

Reports must match the saved PNR/transaction, available item reference, Issued/Ticket state, original total and each passenger's identity, unique ticket numbers and original fare breakdown. Triplover's previously evidenced CNN/CHD ticket-label compatibility is bound to the saved passengers and verified ticket numbers. Returned fare-breakdown groups and flight segments, when supplied, are checked against the accepted booking. Reissued, refunded/cancelled or changed-fare/itinerary cases are not accepted by this initial confirmed-report contract.

The public response keeps the report shape, without an `item1`/`item2` envelope:

- `ticketInfo`: lifecycle/timing, journey type, PNRs, platform identities and accepted selling `ticketingPrice`.
- `passengerInfo`: passenger name/type, available gender/date of birth, ticket numbers and fare breakdown with accepted selling `totalPrice` and derived discount.
- `fareBreakdown`, when supplied: verified passenger-type counts and selling fares.
- `segments`, when supplied: verified itinerary with selected flight metadata.

No new markup calculation occurs. Values come from the booking's accepted selling snapshot; decimal scale in JSON may differ while monetary value is unchanged. The report's supplier-agent account fields, reference logs, internal IDs/transaction numbers, supplier markup, supplier payment assertions, document/contact copies, unrelated financial sections and unknown fields are excluded. The report is a verified public projection, not a raw back-office export. Existing passenger document/contact data remains in the booking record.

## Failures and stored evidence

- **403:** missing ticketing permission or supplier servicing disabled.
- **404:** unknown/foreign booking or reference.
- **409:** no verified issued ticket, ambiguous reference/transaction or invalid configuration context.
- **422:** unsupported status filter.
- **502:** supplier read failed or report failed verification. Raw failure details are not exposed.
- **504:** supplier timeout.

Migration `0017_ticket_reports.sql` creates append-only report evidence with booking/client ownership, request/receipt timestamps, raw response, verification flag and public projection. Valid and rejected received responses are retained with a redacted audit event. Transport failures do not create a report snapshot. Ticket issue/resolution state is never changed by a report lookup. Long-term private-data retention remains part of the existing production readiness work.

## Verification and rollout

The implementation is covered by supplier GET/retry/path-encoding tests, pricing/identity tests, full database integration and read-only Triplover UAT checks against the existing BS return ticket. See [evidence](evidence/TICKET_REPORT_2026-09-11.md).

Apply migration 0017 through the normal backup/migrate workflow before serving the new binary. This report increment was released as `cd3f2b5` on 2026-09-11, with migration 0017, successful readiness and all three report routes verified. Production ticket mutation remains blocked; Direct Issue and Cancel remain outside this increment.

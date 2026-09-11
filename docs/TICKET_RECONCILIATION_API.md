# Uncertain ticket Issue reconciliation

`POST /api/bookings/{bookingUuid}/ticket/reconcile` performs supplier reads to recover ticket evidence after an Issue timeout/lost response. Requires both machine `booking` and `ticketing` permissions and booking ownership.

- **200:** an existing verified ticket receipt, or a receipt reconstructed from independently verified PNR/report evidence.
- **202:** insufficient evidence; the issue remains blocked and must not be retried upstream.
- **403/404:** permission/servicing denied or unknown/foreign booking.
- **409:** Issue is still within the initial five-minute pending window, required references are missing, or the local booking is not a verified accepted Hold.

Replays of already resolved issues do not contact suppliers. Pending reservations become eligible for a read-only check after five minutes; outcome_unknown reservations can be checked immediately. No reconciliation ever deletes/releases an issue reservation, changes its original response or sends NewTicket/Book/Cancel. A supplier not-found/failure/Booked result is not proof that ticketing never occurred.

## Evidence required

The original supplier servicing switch must be enabled. Search participation and ticket mutation enablement need not be enabled. Supplier currency must match the accepted booking.

1. Read PNR using the reservation's stored six supplier references. Require success, matching PNR, Ticketed status and no conflicting available echoed references.
2. Read the Confirmed report using the stored original transaction. Require Issued/Ticket, explicit completion, parseable issue date, complete itinerary, all booked passengers, unique valid ticket numbers and original passenger/aggregate fares. Any supplied breakdown groups must match too.
3. Any ticket numbers already present in PNR or a captured successful Issue response must agree with the report. Conflicting evidence leaves the issue unresolved. Recheck any late worker response while holding the issue row lock before publication.
4. Create a public ticket receipt with platform references and verified report passenger/ticket data. No supplier ticketCodeRef or original NewTicket response is invented. The public ticketCodeRef remains the platform issue UUID. Supplier selling/report pricing uses the existing accepted snapshot, with no additional markup.

Recovery uses an independent report verifier; it does not create a fake saved receipt to pass the report endpoint's existing-ticket checks. Original supplier evidence and original pending/unknown state stay immutable. An append-only verification gives the effective issued state used by ticket retrieval, Book's X-Ticket-State, report lookup and idempotent NewTicket replay.

## Admin operations

- `GET /admin/ticket-issues`: oldest 100 unresolved issue reservations, with booking/public reference, client, supplier, eligibility and latest check summary. Uses a human Admin session; machine tokens are rejected.
- `POST /admin/bookings/{bookingUuid}/ticket/reconcile`: same verification rules with administrator audit identity. Returns an issued/unresolved summary without exposing raw ticket/report evidence.

These are APIs available through Swagger; the existing booking reconciliation screen is not extended in this increment. Unresolved cases without complete verifiable evidence remain for supplier support. No manual bypass, automatic re-issue, refund/void or direct-issue action is added.

## Persistence and limits

Migration `0018_ticket_reconciliation.sql` stores append-only PNR/report evidence, result/error summary, actor and timestamps. Verification and audit commit atomically. Concurrent client/Admin/saved-response verification serializes on the issue row; one verification wins and none can overwrite it. A late pending worker may retain its original outcome without erasing already verified evidence.

Supplier read timeout/failure is retained as an insufficient-evidence attempt and returns the normal unresolved 202 response. Database failure can prevent saving evidence; clients can safely repeat reconciliation because it is read-only upstream. There is no background scheduler or indefinite retry loop. Admin queue pagination beyond the oldest 100, a dedicated UI, and evidence retention policy remain separate work.

Released as `15e8e58` on 2026-09-11 through the existing backup/migration workflow, including migration 0018. Production readiness, all three routes and unauthenticated access rejection were verified. No supplier calls or private UAT data transfer occurred during deployment. See [verification evidence](evidence/TICKET_RECONCILIATION_2026-09-11.md).

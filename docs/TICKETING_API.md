# Held booking Ticket Issue

`POST /api/ticket/NewTicket` confirms an existing verified Hold. This release supports Triplover UAT execution only. Production payment/credit authorization and Direct Issue remain unavailable.

Send a machine token with both `booking` and `ticketing` permissions, `Idempotency-Key` (1–128 visible ASCII characters), and the same platform references used by `/api/pnr`:

```json
{
  "PNR": "ABCDEF",
  "BookingRefNumber": "ABCDEF",
  "UniqueTransID": "<platform search UUID>",
  "PriceCodeRef": "<accepted price UUID>",
  "ItemCodeRef": "<platform offer UUID>",
  "BookingCodeRef": "<platform booking UUID>"
}
```

Obtain these from the saved Hold response, including through `GET /api/bookings/by-reference/{reference}`. The public `STR…` reference is a lookup key; it does not replace the six request fields. Unknown fields, including direct-issue intent or fare-change overrides, are rejected.

## Eligibility and execution

- Owned, automatically verified `held` booking; both Search and accepted RePrice must have `bookable=true`. Manually resolved/unknown bookings cannot be issued.
- Accepted selling fare is retained without another markup calculation. Search/quote expiry does not expire an existing Hold.
- Database servicing and ticketing controls and transport ticketing configuration must be enabled. The real adapter additionally requires the exact approved HTTPS Triplover UAT Search and API hosts. Application environment must be `uat` (or `test` with mocked transports). Neither these switches nor this implementation authorize production commerce.
- A fresh supplier PNR lookup must verify the original references, `Booked` status, no ticket evidence and sufficient deadline. Preflight evidence is saved and must be less than 30 seconds old when reserving dispatch.
- RFC3339 deadlines use their explicit offset. For the documented offset-free `MM/DD/YYYY HH:MM:SS`, use the earliest possible UTC deadline across offsets (UTC+14) as a conservative lower bound. This does not assert a supplier timezone. Missing/expired/too-close deadlines fail closed; the dispatch timeout plus 30 seconds must fit before that bound.
- The supplier receives only the six stored references. No `TicketWithNewFare`, partial-payment or commission override is sent. There is no automatic retry, including after 401/5xx, timeout or a lost response.

## Results and retrieval

- **200:** verified ticket evidence, with `ticketInfoes[].ticketNumbers[]`. Passenger identities/counts, unique numeric ticket numbers, PNR, available echoed references and any flight/fare payload are validated. Any flight fare is projected using the already accepted selling snapshot. Opaque ticket references become a platform issue UUID.
- **202:** durable `pending` or `outcome_unknown`, with `issueId`, `bookingId` and `requiresReconciliation=true`. Treat supplier business failure as potentially unresolved too. Never send another supplier issue request to try to fix it.
- **403:** missing permission, production restriction or supplier gate.
- **409:** held/readiness/deadline checks failed, or idempotency key belongs to another request.
- **502/504:** preflight PNR lookup failed/timed out; no issue reservation or dispatch.

`GET /api/bookings/{id}/ticket` returns the same saved issue result, owner-scoped and requiring ticketing permission. Original Book replies remain immutable. Book replay and booking retrieval expose `X-Ticket-State` when an issue reservation exists, alongside the existing `X-Booking-Reference`.

A database-unique reservation prevents more than one Issue per booking even with concurrent/different keys. Same request replays return saved results without another PNR/Issue dispatch. Reusing a key for another booking returns 409. The completion worker persists received outcomes even when the requesting HTTP connection closes. A process crash can leave `pending`; it stays blocked for operational investigation. Terminal evidence and original reservation fields cannot be edited or deleted by normal SQL.

## Migration and limits

Apply migrations `0015_held_ticketing.sql` and `0016_ticket_verification.sql` through the established backup/migrate workflow before serving this version. Existing release deployment grants cover new tables automatically. Raw responses/preflight and accepted original/selling data stay in the database, with redacted audit events.

Resolution of uncertain issues without a verifiable captured success, an Admin ticket-resolution UI, ticket reports, production commercial controls and broader supplier acceptance remain unfinished. PNR remains a read-only evidence endpoint; it cannot silently turn an uncertain issue into a successful ticket record. Do not interpret a preserved Hold reply as current live supplier status; read `/ticket` and PNR.

The opt-in `uat_held_ticket` example is restricted to the retained local BS return UAT database, requires an explicit environment guard and never calls Book. Back up that database before applying this migration. It temporarily scopes local ticket permissions/controls and removes its token/restores settings after the probe. Private evidence is written with restricted permissions under `.local/evidence/`.

## Captured-success verification

`POST /api/bookings/{id}/ticket/verify` revalidates the original saved supplier Issue response without sending any supplier request. Both booking and ticketing permissions and ownership are required. A complete successful response that passes the current verifier yields an append-only verification record and HTTP 200. The original issue outcome and raw evidence remain immutable. Insufficient/missing evidence returns 409; pending remains 202. Ticket retrieval, Issue replay and `X-Ticket-State` then reflect the verified result. This endpoint cannot manufacture ticket numbers or resolve a timeout without a captured success.

Triplover UAT evidence shows a booked CNN child returned as CHD in ticket passenger metadata, while the CNN flight passenger count and fares are unchanged. Accept this directed label compatibility only for Triplover, matching names and verified unchanged flight counts/fares. Preserve the returned CHD label in public ticket metadata. Other supplier/type mismatches remain rejected.

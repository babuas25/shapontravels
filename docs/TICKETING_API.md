# Held booking Ticket Issue

`POST /api/ticket/NewTicket` confirms an existing verified Hold. Held-ticket issuance supports configured FirstTrip, TakeOff and Triplover endpoints in UAT and production. Supplier capability flags, database permissions and wallet authorization control dispatch. Direct Issue remains unavailable.

The current local implementation follows **Search → RePrice → local price acceptance → Book → NewTicket**. NewTicket does **not** call `/api/pnr` or require a successful PNR lookup. This follows the supplier PDF v2.0, §4.7 (pages 26–27); §4.9 describes PNR as a separate live status/deadline refresh. See the [review and verification record](evidence/TICKETING_WITHOUT_PNR_2026-09-15.md).

Send a machine token with both `booking` and `ticketing` permissions, `Idempotency-Key` (1–128 visible ASCII characters), and these platform references:

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
- The supplier’s `*_TICKETING_ENABLED=true` flag and database `ticketing_enabled=true`, `servicing_enabled=true` controls must all be enabled. The configured HTTPS endpoints and matching account credentials select UAT or production; `APP_ENV` does not independently block issuance. Wallet ownership, active account, accepted amount, currency and sufficient funds remain mandatory.
- Validate the saved successful Book response (`bookingStatus=Created`, required references, no ticket evidence) and recheck the local held state under the dispatch lock. Missing or failed PNR observations do not block issue. No PNR request is sent and there is no 30-second PNR freshness requirement.
- If a separately requested, previously verified PNR observation exists, its references must agree and its status must be `Booked` or `Created`, with no ticket evidence. The status endpoint uses the same held-status rule. Issue reads the most recent verified observation by request time from the immutable history added in migration 0028. A later failed lookup cannot erase known cancelled/ticketed evidence; an older response completing late cannot supersede a newer verified response. Its `lastTicketTime` supersedes Book's `ticketingTimeLimit`, even if missing. This is saved evidence, not a claim of current supplier status.
- RFC3339 deadlines with an explicit offset are checked locally: the dispatch timeout plus 30 seconds must fit before the deadline. Missing, empty, unsupported or offset-free deadlines are retained verbatim and left for the supplier's NewTicket validation. Book may legitimately return an empty deadline; observed Book and PNR date formats differ. The backend does not invent a timezone or require a PNR call to obtain one. Without a live PNR refresh, a supplier-side cancellation or shortened deadline may first be discovered by NewTicket.
- The supplier receives only the six stored references. No `TicketWithNewFare`, partial-payment or commission override is sent. There is no automatic retry, including after 401/5xx, timeout or a lost response.

## Results and retrieval

- **200:** verified ticket evidence, with `ticketInfoes[].ticketNumbers[]`. Passenger identities/counts, unique numeric ticket numbers, PNR, available echoed references and any flight/fare payload are validated. Surname-only passengers accepted by Book also match in ticket/report verification: their saved empty given name must match exactly and the surname must be nonempty. Any flight fare is projected using the already accepted selling snapshot. Opaque ticket references become a platform issue UUID.
- **202:** durable `pending` or `outcome_unknown`, with `issueId`, `bookingId` and `requiresReconciliation=true`. Treat supplier business failure as potentially unresolved too. Never send another supplier issue request to try to fix it.
- **403:** missing permission or disabled supplier gate.
- **409:** saved hold/evidence checks or a known explicit-offset deadline check failed, or the idempotency key belongs to another request.

PNR availability cannot cause a 502/504 from an issue preflight because that network call no longer exists. A NewTicket timeout, transport/authentication failure or supplier rejection still becomes 202 `outcome_unknown` after reservation; it is never treated as success or automatically retried.

`GET /api/bookings/{id}/ticket` returns the same saved issue result, owner-scoped and requiring ticketing permission. Original Book replies remain immutable. Book replay and booking retrieval expose `X-Ticket-State` when an issue reservation exists, alongside the existing `X-Booking-Reference`.

A database-unique reservation prevents more than one Issue per booking even with concurrent/different keys. Same request replays return saved results without supplier dispatch. Reusing a key for another booking returns 409. Issue and Cancel reservations remain mutually exclusive. The completion worker persists received outcomes even when the requesting HTTP connection closes. A process crash can leave `pending`; it stays blocked for operational investigation. Terminal evidence and original reservation fields cannot be edited or deleted by normal SQL.

## Migration and limits

Apply migrations `0015_held_ticketing.sql` and `0016_ticket_verification.sql` through the established backup/migrate workflow before serving this version. Existing release deployment grants cover new tables automatically. Raw responses/preflight and accepted original/selling data stay in the database, with redacted audit events.

**Migration `0028_booking_pnr_observations.sql` is required before serving this reviewed build.** It preserves PNR observations independently of the latest-attempt display fields and backfills whichever saved observation is still available. Previously overwritten historical evidence cannot be reconstructed. The migration was tested only in disposable databases; it has not been applied to working or production databases.

New `flight_ticket_issues.preflight` JSON records `source=saved_booking`, the original Book response, any separately verified PNR observation, the raw selected deadline, its source, `deadlineCheck` and the local check time. Existing PNR-shaped evidence remains immutable; retrieval, verification and recovery do not depend on either preflight shape. This is an audit of local checks, not fabricated PNR success.

Report-backed uncertain Issue reconciliation is now implemented locally; see [reconciliation API](TICKET_RECONCILIATION_API.md). A dedicated Admin ticket-resolution UI, production commercial controls and broader supplier acceptance remain unfinished. Ticket report APIs were released separately. PNR remains a read-only evidence endpoint; it cannot silently turn an uncertain issue into a successful ticket record. Do not interpret a preserved Hold reply as current live supplier status; read `/ticket` and PNR.

Only the automatic PNR dependency of held-ticket issue is removed. Explicit PNR/status reads, cancellation's PNR validation, and uncertain-ticket reconciliation remain separate operations. Recovery after a lost NewTicket response still needs adequate evidence; an unavailable PNR endpoint can prevent automatic recovery and requires operational investigation. It never permits a second issue attempt. The portal ticket flow uses the same native issue service and eligibility checks; see [Portal ticket wallet](PORTAL_TICKET_WALLET.md).

The opt-in `uat_held_ticket` example is restricted to the retained local BS return UAT database, requires an explicit environment guard and never calls Book. Back up that database before applying this migration. It temporarily scopes local ticket permissions/controls and removes its token/restores settings after the probe. Private evidence is written with restricted permissions under `.local/evidence/`.

## Captured-success verification

`POST /api/bookings/{id}/ticket/verify` revalidates the original saved supplier Issue response without sending any supplier request. Both booking and ticketing permissions and ownership are required. A complete successful response that passes the current verifier yields an append-only verification record and HTTP 200. The original issue outcome and raw evidence remain immutable. Insufficient/missing evidence returns 409; pending remains 202. Ticket retrieval, Issue replay and `X-Ticket-State` then reflect the verified result. This endpoint cannot manufacture ticket numbers or resolve a timeout without a captured success.

Triplover UAT evidence shows a booked CNN child returned as CHD in ticket passenger metadata, while the CNN flight passenger count and fares are unchanged. Accept this directed label compatibility only for Triplover, matching names and verified unchanged flight counts/fares. Preserve the returned CHD label in public ticket metadata. Other supplier/type mismatches remain rejected.

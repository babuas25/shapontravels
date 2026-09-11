# Hold Booking API — implementation and limits

User-approved policy: **hold booking requires no payment, balance or credit check**. The earlier deny-by-default commercial authorizer has been removed. A valid client `booking` permission, supplier database Search/booking enablement and the transport `<SUPPLIER>_BOOKING_ENABLED=true` flag permit a verified hold request, subject to the existing accepted-price, passenger and reference checks. This policy does not authorize instant purchase or ticket issue.

Current Direct Issue behavior is described in [Direct Issue](DIRECT_ISSUE.md): an offline implementation requires explicit intent, both non-holdable flags and dual permissions. A mismatched mode returns `BOOKING_MODE_MISMATCH`; real execution remains disabled. Historical Hold-only evidence below predates that increment.

## POST /api/Book

Machine permission: `booking`. Required header: `Idempotency-Key` (1–128 printable ASCII characters, no spaces). References must come from the latest owned, unexpired, explicitly accepted public RePrice:

```json
{
  "uniqueTransID": "<platform Search UUID>",
  "itemCodeRef": "<platform offer UUID>",
  "priceCodeRef": "<accepted platform pricing UUID>",
  "passengerInfoes": [{
    "nameElement": {"title":"Mr","firstName":"Test","lastName":"Passenger"},
    "gender":"Male",
    "passengerType":"ADT",
    "dateOfBirth":"1985-01-01",
    "documentInfo": {
      "documentNumber":"TEST12345",
      "expireDate":"2030-01-01",
      "issuingCountry":"BD",
      "nationality":"BD"
    },
    "contactInfo": {
      "phone":"1700000000",
      "phoneCountryCode":"+880",
      "email":"test@example.invalid",
      "countryCode":"BD"
    }
  }],
  "directIssueIntent":false,
  "taxRedemptions":[],
  "commissionOnTaxes":[]
}
```

Supported passenger fields are documented in OpenAPI. This initial schema supports passport details, contact details, optional middleName/cityName/isLeadPassenger, and empty/omitted documentType. Unknown fields, document uploads, ancillaries, agent overrides and nonempty tax redemptions are rejected rather than forwarded without an established contract. Empty/omitted commission input uses saved RePrice commissions; nonempty input must match saved data exactly.

Validation checks passenger count/type, DOB age on first departure, exact child-age multiset from Search, name/contact/document syntax, passport expiry through the final departure, and duplicate passenger identity. ADT/CHD/CNN/INF bands follow the local supplier documentation; INS is not supported by the current Search request. These checks are not destination-specific passport/visa/admissibility advice. At most nine passengers are accepted.

Only the saved supplier connection and refreshed **supplier** RePrice references are sent upstream. Client audience/agent, current supplier Search/booking enablement, availability epoch and currency must match. The production transport additionally requires its existing `<SUPPLIER>_BOOKING_ENABLED` environment flag; all remain unchanged. No additional payment/credit authorization is required for holds.

## Durable dispatch and retries

Before contacting a supplier, the endpoint commits a `flight_bookings` record with state `pending`, client-scoped key, canonical request hash, exact supplier request, quote linkage and a redacted audit event. Each new client-scoped idempotency key represents a separate intentional booking. The same offer, pricing version and passengers may be booked again with a new key; migration 0009 removes the earlier unique-offer restriction. Per-client reservation and offer locks serialize concurrent requests; locks are released before network work. Version/expiry are rechecked after lock acquisition.

The mutation transport is separate from the read-retry loop. It sends Book at most once per reserved attempt: no Book resend after 401, 5xx, timeout, malformed response or connection loss. Authentication may obtain a token before the first Book, but does not replay a rejected Book. This is not a supplier-level exactly-once guarantee.

Same key + same normalized payload returns the stored outcome (200) or unresolved status (202), even after the quote expires. Same key + different payload gives 409 `IDEMPOTENCY_KEY_REUSED`. Another key starts a new independently authorized booking, not a retry. This also applies while a previous intent is pending or unresolved: its state remains separate and is never cleared or resent. Clients must retain the same key for network retries and generate a new key only for an explicit new user action. Supplier restrictions may still reject a repeated booking.

An HTTP disconnect does not cancel the detached outcome-persistence worker. A process crash or database failure after reservation can leave `pending`; it is never automatically dispatched again. Reservation and supplier execution are not an atomic distributed transaction. After an ambiguous outcome, system recovery uses the same key/booking ID and reconciliation. It must not automatically generate another key; a user can separately initiate a new booking.

Previous bookings do not block RePrice or acceptance for a new booking intent. Every existing booking retains its original price_id; new intents still require the latest unexpired accepted revision.

## Outcome and prices

A supplier response is returned as a verified hold only when it reports success, `bookingStatus=Created`, a nonempty PNR and bookingCodeRef, and no ticket evidence. PNR, supplier references, raw response, deadline, public response and outcome audit are persisted before responding 200.

If flightInfo is present, its passenger counts, itinerary, currency (when present) and original monetary values must match the accepted RePrice snapshot. Existing total/discount fields receive the stored accepted selling amounts; markup is never calculated a second time. A documented PNR-only success without flightInfo can be recorded as held, but does not independently reconfirm price fields that the supplier omitted. Unsupported top-level booking price shapes remain unresolved rather than expose supplier fare as selling fare.

Observed UAT `flightInfo` may omit its aggregate `totalPrice`, `basePrice` and `taxes`. Missing fields are preserved as missing; present fields must be numeric and match the accepted original quote (explicit null is not treated as omission). Passenger-count, per-passenger and single-component total/base/tax/AIT comparisons remain mandatory. Only existing selling total/discount fields are updated; an absent aggregate total is not invented.

Observed domestic UAT Book may also omit/null the segment cabin label even when RePrice supplies Economy. Missing/null/empty cabin labels do not invalidate otherwise identical itinerary/RBD evidence; the supplier's missing label remains unchanged. Two conflicting explicit cabin labels, flight details or booking classes still reject the held response. This does not automatically rewrite previously saved unknown outcomes.

Any timeout, transport error, supplier business failure, changed/unverifiable price, malformed success or unexpected ticket evidence becomes `outcome_unknown` with HTTP 202. Even `isSuccess=false` is conservatively retained for reconciliation, not assumed to prove no supplier side effect. Existing response evidence is retained privately; raw supplier error messages are not returned. No automatic cancellation is attempted.

Unresolved response:

```json
{"bookingId":"<platform booking UUID>","state":"outcome_unknown","requiresReconciliation":true}
```

Successful response retains the supplier item1/item2 shape and public PNR. Platform bookingCodeRef is the booking ID. Search/offer/price identities are bound by field even if a supplier reuses one opaque string in multiple reference fields.

## Status and reconciliation

- `GET /api/bookings/{id}`: owner-only local stored outcome. No supplier mutation or network call.
- `POST /api/bookings/{id}/reconcile`: owner-only read-side PNR lookup, requiring `booking` permission and supplier `servicing_enabled`. Uses known PNR/booking refs and the stored refreshed transaction/price/item refs. No guessed references or Book fallback.

If a timeout provided no PNR/bookingCodeRef, reconciliation returns 409 `MANUAL_RECONCILIATION_REQUIRED`; the documented PNR API requires those fields. An authorized supplier/support investigation is then necessary. The Admin workflow below can record an evidence-backed manual outcome; it never clears an idempotency key or redispatches Book.

PNR read evidence is stored separately. `X-Booking-State` reflects the local state and `X-Manual-Resolution-Required` remains true for an unresolved booking. A successful PNR read alone does not clear a pricing discrepancy, unexpected ticket issue, or duplicate-booking risk. The original intent record remains in place; there is no automatic state promotion, retry, cancellation or ticketing. Separate intentional keys are not blocked.

## POST /api/pnr — public live status lookup

Requires the owner's machine token with `booking` permission. Use the exact supplier-compatible field casing below, but submit **platform** UUIDs from the saved public booking/price response:

```json
{
  "PNR": "<public booking PNR>",
  "BookingRefNumber": "<same public booking PNR>",
  "UniqueTransID": "<platform Search UUID>",
  "PriceCodeRef": "<platform accepted price UUID used by this booking>",
  "ItemCodeRef": "<platform offer UUID>",
  "BookingCodeRef": "<platform booking UUID>"
}
```

All six fields must match the client-owned booking. Unknown/foreign booking IDs return 404; mismatched references return 422 `BOOKING_REFERENCE_MISMATCH`, before any supplier request. Unknown body fields are rejected. Expired Search/price references and disabled Search participation do not prevent servicing an existing booking. Supplier `servicing_enabled` is required; new-booking enablement is not required for this read.

This endpoint and `/api/bookings/{id}/reconcile` share the saved-reference lookup. Only the original supplier is contacted. Successful responses preserve the supplier `item1`/`item2` envelope, `bookingRef` PNR mirror, and null references. Search/offer/price/booking identifiers are bound by field to their platform IDs even when the supplier reuses a single string for all reference types. A wrong PNR, contradictory echoed reference, absent status, or supplier business failure returns 502 `SUPPLIER_RECONCILIATION_FAILED`; transport errors return 502 `SUPPLIER_READ_FAILED`, and the application deadline returns 504 `SUPPLIER_TIMEOUT`.

A verified PNR read replaces stored `ticketing_time_limit` with `lastTicketTime` in the documented `MM/dd/yyyy HH:mm:ss` form. A missing, null, empty or unparseable latest deadline clears the old stored deadline instead of treating it as current. The response retains the supplier's original value. The timezone remains unverified, so this is raw deadline evidence, not authorization to issue or a UTC expiry calculation. Failed/mismatched responses cannot replace the stored deadline. Overlapping observations are ordered by database dispatch timestamp; an older lookup cannot overwrite a newer persisted observation. Evidence/deadline updates and a redacted `booking.pnr` audit event commit atomically. The Admin workflow in migration 0013 additionally records whether the latest supplier evidence passed validation, so failed/mismatched responses are not displayed as verified status.

`X-Booking-State` remains the local booking outcome. `X-Manual-Resolution-Required` is true for unresolved bookings, a supplier status other than `Booked`, or ticket-number evidence. PNR success does not promote an uncertain booking to held or clear a pricing discrepancy. The existing local status/idempotent Book response remains the original stored outcome; use `/api/pnr` for current supplier status/deadline. Manual resolution is provided by the Admin workflow below; Cancel remains unfinished.

Current execution restriction: any actual Hold/Book or Issue validation must use **Triplover UAT only**, never production. This endpoint change was verified using mocks and a disposable local PostgreSQL database; it does not establish UAT PNR availability or create a booking.

## Admin manual reconciliation and screen

Open `/admin/reconciliation` on the API origin. The Bengali screen provides existing Admin/Super Admin login, unresolved/resolved lists with pagination, booking details, “Status আবার চেক করুন”, and a form for the supplier-confirmed outcome, reason, evidence and supplier case/confirmation reference. It displays who recorded each decision and when. An explicit checkbox confirms that the administrator has verified the outcome with the supplier. The screen is a narrow user-approved addition to the original backend-only scope.

The HTML shell is public and contains no booking data. All data/actions require the existing separate `sta_` Admin session; machine tokens cannot use them, and Admin sessions still cannot use commercial `/api/Book`. Session tokens stay only in page memory; reload requires login. The page uses same-origin requests, a restrictive CSP, no third-party scripts or assets, and text-only rendering for all supplier/admin content. Passport documents, passwords and full passenger payloads must not be entered into evidence. No upload or external URL fetch is implemented.

| Endpoint | Behaviour |
|---|---|
| `GET /admin/bookings` | Up to 50 unresolved records plus `nextCursor`; use `before` for subsequent pages and `resolved=true` for resolved records |
| `GET /admin/bookings/{id}` | Booking summary, current version, resolution eligibility and immutable decision history; no raw passenger/supplier payload |
| `POST /admin/bookings/{id}/recheck` | Same saved-reference PNR read as the client lookup, with an Admin audit actor; never Book/Cancel/Issue |
| `POST /admin/bookings/{id}/resolve` | Atomically record verified manual outcome, update local status and append redacted audit |

Resolve request:

```json
{
  "outcome": "held",
  "reason": "Supplier support confirmed the held reservation",
  "evidence": "The supplier portal and support confirmation agree",
  "supplierCaseRef": "<supplier support case or confirmation reference>",
  "expectedUpdatedAt": "<updatedAt from the current detail response>",
  "confirmedWithSupplier": true
}
```

Outcomes: `held`, `not_created`, `issued`, `cancelled`. These record a supplier-confirmed fact; selecting an outcome does not execute that operation. A reason (10–2,000 characters), evidence (10–4,000), reference (3–200), current version and explicit supplier confirmation are mandatory. The browser handles IDs/versioning; the administrator fills ordinary labelled fields.

Only `outcome_unknown` and `pending` records older than five minutes can be resolved. The five-minute minimum exceeds the existing maximum 120-second supplier deadline and prevents ordinary in-flight requests being closed prematurely. Row locking and `expectedUpdatedAt` reject stale/concurrent decisions; already resolved records return `ALREADY_RESOLVED`. Each decision and redacted audit event commit in one transaction. Immutable decision history retains the private evidence, administrator and timestamp. A resolved record has state `manually_resolved`, not the automatically verified `held` state.

Owner-only local status and replay of the original idempotency key return HTTP 200:

```json
{"bookingId":"<platform UUID>","state":"manually_resolved","requiresReconciliation":false,"manualOutcome":"held"}
```

This platform resolution response is not a fabricated supplier booking success or price confirmation. Private reason/evidence/admin details are not included. Original request hash, supplier attempt and quote ownership remain unchanged; key reuse never redispatches Book. Manual `held` does not invent missing PNR/reference data, certify an old price, or enable ticketing/Cancel. Missing-reference servicing still requires supplier assistance.

If an exceptionally delayed Book worker finishes after manual resolution, its response is stored in an immutable late-outcome record, the earlier decision is preserved, and the booking reopens as `outcome_unknown` with a redacted `booking.late_outcome` audit event. The UI flags the new evidence for review. An administrator can make a fresh, version-checked decision; prior history is never overwritten. No automatic Book retry occurs.

Migration `0013_manual_reconciliation.sql` adds the manual state, immutable resolution/late-response tables and indexes. Existing release migration/grants machinery covers these tables/sequences. No new default account, credential change, production migration or deployment is part of this implementation. Storage/access/retention protection for private evidence follows the booking-data policy and still needs full production readiness verification. Actual supplier validation remains restricted to Triplover UAT; local mocks do not establish live supplier acceptance.

## Storage and rollout

Migration 0008 adds client/offer/price ownership constraints and dispatch records; migration 0009 allows multiple intents per offer while retaining client/key uniqueness. The records include original/public outcomes, reconciliation evidence and PNR/deadline fields. Sensitive passenger requests and raw supplier outcomes are stored in the application's private database; they are not logged. Production storage encryption/access/retention remain deployment considerations for real passenger data.

No migration was applied to the user's running database and no server was restarted. Mock/database tests use disposable local databases and synthetic passengers. No live Book was executed as part of the payment-policy change. At that stage instant purchase was unsupported. The later [Direct Issue increment](DIRECT_ISSUE.md) adds an authorized offline implementation with real execution disabled.


A quote whose offer has subsequently received a classified fare/session rejection cannot be used for a new booking: HTTP 409 `REPRICE_REQUIRED`. Migration 0011 adds this guard. A fresh successful RePrice and acceptance are required; existing idempotent booking-result replays remain unchanged. This guard was mock-tested only; the 2026-09-10 prebooking work did not execute any live Book or ticketing operation.

## Public reference retrieval — 2026-09-11

The agreed public reference is `STR` + GDS PNR + airline PNR, without separators: `STR8FE94RKECOCE`. It is stored against the platform booking UUID, not parsed to authorize supplier access.

```http
GET /api/bookings/by-reference/STR8FE94RKECOCE
Authorization: Bearer <client machine token>
```

Requires the owning client's `booking` permission. Returns the same saved response as `GET /api/bookings/{id}`: 200 for held/manually resolved, 202 for unresolved. This is saved order retrieval, not a fresh supplier PNR check. Existing UUID reconciliation/PNR APIs remain the live-check path.

Book responses, idempotent replays, and both saved retrieval routes include `X-Booking-Reference` when available; supplier-shaped JSON remains unchanged. Admin queue/detail expose nullable `publicRef`, displayed as Order Ref.

Migration 0014 derives references from successful saved Book evidence, with verified matching PNR evidence as fallback. Both locators must be six uppercase alphanumeric characters. Repeated identical airline PNRs across legs are one locator. Missing, malformed, or multiple distinct airline PNRs leave the reference null; continue using the booking UUID. Once established, the reference is immutable even if subsequent supplier evidence changes.

Malformed/unknown/foreign references return 404. PNR pairs can be reused: multiple matching bookings belonging to the same client return 409 `BOOKING_REFERENCE_AMBIGUOUS`; retrieve by UUID in that case. Supplier outcome persistence is never rejected solely because another booking has that pair. A reference is an identifier, not an authentication secret.

The retained isolated BG UAT booking was migrated and both reference/UUID retrievals returned 200 with identical saved JSON. No supplier call was made during this verification. See [reference evidence](evidence/PUBLIC_BOOKING_REFERENCE_2026-09-11.md).

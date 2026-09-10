# Hold Booking API — implementation and limits

User-approved policy: **hold booking requires no payment, balance or credit check**. The earlier deny-by-default commercial authorizer has been removed. A valid client `booking` permission, supplier database Search/booking enablement and the transport `<SUPPLIER>_BOOKING_ENABLED=true` flag permit a verified hold request, subject to the existing accepted-price, passenger and reference checks. This policy does not authorize instant purchase or ticket issue.

Direct issue is intentionally unsupported in this stage. Either Search or RePrice having `bookable != true`, or a caller requesting `directIssueIntent=true`, yields `DIRECT_ISSUE_UNSUPPORTED`. Ticketing permission does not enable direct issue here. No NewTicket/Cancel implementation was added, no supplier booking was performed during development, and no running production settings were enabled.

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

Any timeout, transport error, supplier business failure, changed/unverifiable price, malformed success or unexpected ticket evidence becomes `outcome_unknown` with HTTP 202. Even `isSuccess=false` is conservatively retained for reconciliation, not assumed to prove no supplier side effect. Existing response evidence is retained privately; raw supplier error messages are not returned. No automatic cancellation is attempted.

Unresolved response:

```json
{"bookingId":"<platform booking UUID>","state":"outcome_unknown","requiresReconciliation":true}
```

Successful response retains the supplier item1/item2 shape and public PNR. Platform bookingCodeRef is the booking ID. Search/offer/price identities are bound by field even if a supplier reuses one opaque string in multiple reference fields.

## Status and reconciliation

- `GET /api/bookings/{id}`: owner-only local stored outcome. No supplier mutation or network call.
- `POST /api/bookings/{id}/reconcile`: owner-only read-side PNR lookup, requiring `booking` permission and supplier `servicing_enabled`. Uses known PNR/booking refs and the stored refreshed transaction/price/item refs. No guessed references or Book fallback.

If a timeout provided no PNR/bookingCodeRef, reconciliation returns 409 `MANUAL_RECONCILIATION_REQUIRED`; the documented PNR API requires those fields. An authorized supplier/support investigation is then necessary. No manual override/clear-reservation API is implemented in this stage.

PNR read evidence is stored separately. `X-Booking-State` reflects the local state and `X-Manual-Resolution-Required` remains true for an unresolved booking. A successful PNR read alone does not clear a pricing discrepancy, unexpected ticket issue, or duplicate-booking risk. The original intent record remains in place; there is no automatic state promotion, retry, cancellation or ticketing. Separate intentional keys are not blocked.

## Storage and rollout

Migration 0008 adds client/offer/price ownership constraints and dispatch records; migration 0009 allows multiple intents per offer while retaining client/key uniqueness. The records include original/public outcomes, reconciliation evidence and PNR/deadline fields. Sensitive passenger requests and raw supplier outcomes are stored in the application's private database; they are not logged. Production storage encryption/access/retention remain deployment considerations for real passenger data.

No migration was applied to the user's running database and no server was restarted. Mock/database tests use disposable local databases and synthetic passengers. No live Book was executed as part of the payment-policy change. Instant purchase/direct issue remains unsupported and needs its own design and explicit authorization.


A quote whose offer has subsequently received a classified fare/session rejection cannot be used for a new booking: HTTP 409 `REPRICE_REQUIRED`. Migration 0011 adds this guard. A fresh successful RePrice and acceptance are required; existing idempotent booking-result replays remain unchanged. This guard was mock-tested only; the 2026-09-10 prebooking work did not execute any live Book or ticketing operation.

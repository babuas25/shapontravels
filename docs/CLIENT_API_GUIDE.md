# Client integration guide

Contract reviewed on 18 September 2026. Use this guide with the **running environment's** `/openapi.json` and `/docs/`. Public documentation excludes Admin operations. `Try it out` executes real requests against that environment.

For a held booking's current deadline, explicitly call `/api/pnr` with its saved references. Prefer the latest verified PNR `lastTicketTime` over the original Book `ticketingTimeLimit`. The confirmed named-month format (`19 Sep 2026, 12:44 PM`) is Bangladesh local time: `lastTicketTimeIso` resolves it to `2026-09-19T12:44:00+06:00`, and `lastTicketTimeZone` is `Asia/Dhaka`. Raw Book/replay receipts remain unchanged. A verified missing deadline clears prior deadline authority; do not fall back to an obsolete Book value.

## Authentication and requests

Exchange your issued Client ID and secret:

```http
POST /auth/token
Content-Type: application/json

{"client_id":"<UUID>","client_secret":"<issued secret>"}
```

Success returns `access_token`, `token_type: "Bearer"`, `expires_in: 1800`. Cache the token and coordinate renewal. Send `Authorization: Bearer <access_token>` on commercial requests. `/auth/me` returns your current identity and permissions. Keep secrets and tokens on your server. Managed API clients need active Enterprise API access; tokens cannot grant themselves permissions.

Paths and JSON field names are case-sensitive: `/api/Reprice` differs from `/api/RePrice`. Search/Reprice/Book use camelCase; PNR/NewTicket requests use the exact capitalized fields shown below. Unknown request fields are rejected. Response schemas deliberately allow additional metadata; clients should tolerate unfamiliar response fields and preserve null/absent distinctions.

## Booking journey

| Step | Request | Successful result / next action |
| --- | --- | --- |
| Search | `POST /api/Search` | Offers at `item1.airSearchResponses[]`; retain their platform references. |
| Fare rules | `POST /api/FareRules` | Read rules for the selected complete itinerary. A read failure does not imply a free cancellation policy. |
| Reprice | `POST /api/Reprice` | Refreshed terms, `priceCodeRef`, `isPriceChanged`, `fareBreakdown`; show payable and terms again. |
| Accept | `POST /api/Reprice/accept` with `{"priceCodeRef":"<UUID>"}` | Local acceptance of the latest revision, required even if `isPriceChanged=false`; no inventory is held. |
| Hold | `POST /api/Book` with passenger data and `Idempotency-Key` | Verified hold receipt or unresolved `202`; save `bookingCodeRef` and `X-Booking-Reference`. No wallet debit for Hold. |
| Issue | `POST /api/ticket/NewTicket` with saved booking references and a separate `Idempotency-Key` | Verified ticket receipt or unresolved `202`; ticketing permission, current supplier controls and wallet eligibility apply. |
| Retrieve | `GET /api/bookings/{id}` and `GET /api/bookings/{id}/ticket` | Saved booking/ticket outcome without redispatch. |
| Verify live details | `POST /api/pnr`, `GET /api/bookings/{id}/ticket/report` | Supplier reads; preserve separate booking, ticket and payment states. |

Search permits 1–9 passengers **including infants**, at least one adult, infants no more than adults, up to six routes, and cabin classes 1–5. `childrenAges` must match `childs`, ages 2–11. Airport codes are three uppercase letters. Dates are `YYYY-MM-DD`, nondecreasing and not in the past. For the September UAT audit, all live flights are at least 15 days ahead.

Each Search `directions[routeIndex]` contains alternatives. Select **one complete direction per route**; concatenate its segment references in route/segment order for FareRules and Reprice. Use Search references on those requests. Reprice can return null/missing refreshed segment references: this is valid. Book uses the latest accepted `uniqueTransID`, `itemCodeRef`, `priceCodeRef`; it does not use refreshed segment references.

Local Search/offer validity is ten minutes; a supplier session can expire sooner. Reprice does not extend that window. `410` requires a fresh Search; replace all references together. A rejected fare may require selecting another offer and independently repricing/accepting it.

PNR and held-ticket Issue share this request shape:

```json
{
  "PNR": "<held receipt pnr>",
  "BookingRefNumber": "<same pnr>",
  "UniqueTransID": "<platform search UUID>",
  "ItemCodeRef": "<platform offer UUID>",
  "PriceCodeRef": "<accepted price UUID>",
  "BookingCodeRef": "<booking UUID>"
}
```

`BookingRefNumber` here is the receipt PNR, not the `STR…` public booking reference. Use the public reference on the `/api/bookings/by-reference/{reference}` routes. Detailed passenger fields and validation: [Book contract](BOOKING_API.md). Ticket handling: [held-ticket contract](TICKETING_API.md).

## Imported bookings on the same read endpoints

Airline (`IMP_EXP`), Supplier API (`SUPPLIER_API`) and Manual (`MANUAL`) imports assigned to your canonical agency are available with your existing API token and permissions:

- `GET /api/bookings/by-reference/{STR…}` or `GET /api/bookings/{bookingCodeRef}` returns the normal `BookingReceiptResponse` (`item1` / `item2`).
- `GET /api/bookings/{bookingCodeRef}/ticket` returns the normal `TicketReceiptResponse`, ticket numbers and payment state after confirmation.
- `GET /api/bookings/{bookingCodeRef}/ticket/report` and `GET /api/bookings/by-reference/{STR…}/ticket/report` return the normal `TicketReport` shape.
- `GET /api/B2BReport/AirTicketingDetails/{uniqueTransID}/Confirmed` uses the transaction UUID from the imported receipt.
- `POST /api/pnr` accepts the six references returned by the booking receipt. `POST /api/bookings/{bookingCodeRef}/reconcile` returns the same saved PNR projection.
- `GET /api/pricing/booking/{bookingCodeRef}` returns saved import pricing, preserving the actual payable at import time.

Import reads carry `X-Evidence-Source: saved-import`. PNR/reconcile/report for imports return stored import evidence, **not a fresh supplier query**. Source itinerary details last refreshed by Super Admin are included. Native bookings keep their existing live supplier-read behavior. Import references are stable platform UUIDs; they are not references to a Search/Reprice operation and cannot authorize Book, Issue, cancellation or other supplier mutations.

Access requires an active canonical agency owner linked to the API client, normal API eligibility, and `booking` or `ticketing` permission as for native bookings. Foreign agencies, suspended owners/agencies and unlinked clients receive 404. Import administration remains Super Admin only. Importing or reading a ticket does not enable API access for an account that did not already have it.

The legacy gross flight amounts and the reconciled `fareBreakdown.payable` remain separate. Legacy imports with one fully evidenced fare type can expose a reconciled breakdown using their captured payable; their historical tier is unknown (`tier: null`). Missing fare evidence is never guessed: `flightInfo` may be null, and pricing/report reads requiring that evidence return 409. Ticket associations require matching passenger/ticket counts. The import-success response also includes the same native receipt fields; existing portal workflow metadata remains for frontend compatibility.

## Amounts and response states

For B2B, legacy flight `totalPrice` is **published gross: original base + taxes**. It is not the wallet charge. Search filters summarize this projected gross. Use `fareBreakdown.payable` or `/api/pricing/{kind}/{id}` for final payable (`kind`: `offer`, `reprice`, `booking`). Batch offer pricing is `/api/pricing/offers`. The booking snapshot retains its accepted price even when current rules change.

`fareBreakdown` contains exact two-decimal strings for base, taxes, AIT, service charge, discount, tier adjustment, gross and payable, plus passenger counts. Multiply per-passenger values by their counts before summing. Do not add AIT again to payable. Pricing `commission` is the signed gross-to-payable discount, so `gross = commission + payable`; a negative discount is valid. See [pricing rules](B2B_TIERS.md) for the complete calculation. B2C retains its existing selling-price projection.

Wallet fields ending in `Minor` are **integer strings** (scale 2); `"565800"` means BDT 5,658.00. Use exact decimal/integer arithmetic. Legacy supplier numeric fares, decimal pricing strings and wallet minor-unit strings are intentionally different representations.

`fareBreakdown.gross` is a display amount in the reconciled breakdown and can equal payable when payable exceeds published gross. Use the pricing snapshot's `gross` or the legacy B2B flight total for published base+tax gross; do not assume the two `gross` fields always match. Wallet reads require `wallet:read` permission.

HTTP `202` means the operation needs resolution. It does not prove that no booking/ticket exists. In particular an issued receipt can be returned with payment settlement unresolved. Inspect saved state, `payment`, and `X-Booking-State`/`X-Ticket-State`; never trigger another Issue to resolve a payment discrepancy. Ticket reports have their own schema and an array named `fareBreakdown`; that array is distinct from the reconciled flight `fareBreakdown` object.

Unresolved **Book** responses also carry `reason`, `nextAction`, `statusUrl` and `automaticRetryAllowed: false`. `BOOKING_IN_PROGRESS` with `nextAction: check_saved_status` allows bounded status polling. **Stop automatic polling at `nextAction: contact_support`**: show an awaiting-review result, not an endless processing spinner. This applies to completed `outcome_unknown` and to pending records older than five minutes. `SUPPLIER_DUPLICATE_REPORTED`, `SUPPLIER_REPORTED_FAILURE`, `BOOKING_TIMEOUT`, transport and unverified-response codes distinguish causes without exposing raw supplier text. None certifies non-creation. Historical rows without detailed diagnostics can retain `BOOKING_OUTCOME_UNKNOWN`.

Unresolved **Issue** responses provide the same four diagnostic fields. `TICKETING_IN_PROGRESS` permits bounded saved-status polling; stale pending, supplier failure, unverified response and unknown outcome require `contact_support`. `SUPPLIER_RECORD_LOCATOR_NOT_FOUND` reports a supplier claim, not proof that no ticket exists. A verified issued receipt with incomplete payment settlement carries `WALLET_SETTLEMENT_REQUIRED`; do not issue again or release held funds. Missing retained supplier responses use the general `TICKETING_OUTCOME_UNKNOWN` code because their transport cause is not available. See [ticketing details](TICKETING_API.md).

## Retry and errors

Persist one `Idempotency-Key` per intended Book or Issue: 1–128 visible ASCII characters without spaces. Reuse the same key and unchanged payload after a transport failure. A changed payload with the same key returns `409`; a new key represents a new intended booking. Never create a new key merely to recover an unknown result. Read saved status and use the documented read-only reconciliation/verification endpoints. The server does not resend supplier Book/Issue on timeout, 401 or 5xx.

Backend failures use `{"error":"CODE"}` including malformed JSON, path/query and body-limit errors. Log HTTP status, code and `x-request-id`, without logging credentials or passenger data. Reverse proxies may generate their own non-JSON errors; handle HTTP status even when parsing fails.

| HTTP | Typical action |
| --- | --- |
| 400 / 413 / 415 / 422 | Correct input/content type/size; do not blindly retry. |
| 401 / 403 | Renew credentials if appropriate or resolve permission/enablement. |
| 404 | Resource absent or owned by another client; no foreign-resource disclosure. |
| 409 / 410 | Follow the code: reprice, select another offer, refresh Search, or inspect an existing operation. |
| 429 | `RATE_LIMITED`, `Retry-After: 60`. |
| 503 | `SEARCH_BUSY`, `REPRICE_BUSY`, `AUTHENTICATION_BUSY` carry `Retry-After: 1`; use bounded backoff with jitter. Other 503s can indicate database/supplier availability. |
| 502 / 504 | Supplier read/validation/timeout; mutation outcome must be recovered using the same key and saved status. |

Reprice permits one active read per client and bounded per-process concurrency. Login quotas and CPU capacity are separate for machines and administrators. Operators must configure trusted reverse-proxy IPs correctly; see [authentication](AUTHENTICATION.md).

## Available capabilities

Search, FareRules, Reprice, acceptance, Hold, held-ticket Issue, status, reports, pricing and wallet reads are implemented subject to current permissions/configuration. **Real Direct Issue and real held cancellation remain disabled**; do not offer them to customers as available services. Branded fares, nonempty tax redemption and unsupported ancillary/multiple-component pricing remain guarded. One supplier/route's UAT success does not establish acceptance of every carrier, itinerary or production account.

## Agency requests to Shapon Travels staff

Refund, reissue and void requests use the [Ticket-management API](TICKET_MANAGEMENT_API.md). `POST /api/ticket-management` creates a request in the admin queue; it does not call suppliers. The API supports eligibility, request status and accepting/rejecting staff quotations. Grant `ticket-management:read` and `ticket-management:write` explicitly through API Management. Reissue dates are structured per-route preferences. See the linked guide for JSON examples, idempotency and quotation amounts.

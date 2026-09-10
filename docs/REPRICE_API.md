# RePrice and price acceptance

`POST /api/Reprice` uses a machine token with `search:read`. It reprices one owned, unexpired Search offer through its saved supplier connection. No booking, cancellation or ticketing operation is called.

## Request

```json
{
  "uniqueTransID": "<platform Search UUID>",
  "itemCodeRef": "<platform Search offer UUID>",
  "segmentCodeRefs": ["<platform segment UUIDs from one selected direction per route, in route order>"],
  "brandedFareRefs": "",
  "taxRedemptions": [],
  "commissionOnTaxes": []
}
```

Use the original public Search references on each RePrice request. Choose exactly one complete direction from each Search `directions[routeIndex]` array, concatenate its segmentCodeRefs in segment order, then concatenate routes in request order. The server resolves these public references to the corresponding saved supplier directions. A single-option offer uses the same request as before. Arbitrary supplier references, foreign offers, partial directions, reordered routes/segments, all alternatives combined, extra references and ambiguous matching prefixes are rejected with `OFFER_REFERENCE_MISMATCH` before any supplier call. Nonempty branded fares and tax redemption are unsupported. Commission input may be omitted/empty; the server forwards the original supplier commission data. Nonempty caller commission data must equal that saved value.

The connection must still be Search-enabled at the same availability epoch. Its configured currency must match Search and the refreshed response. Availability/expiry are checked again after the upstream read. RePrice enforces passenger type counts, carrier and route endpoints/continuity. The response must contain exactly one direction per selected route, with the same ordered segment endpoints, airlineCode, flightNumber and departure as selected. Refreshed references and prices may differ. Supplier-confirmed contract (2026-09-10): response segmentCodeRef values may be null or absent. Search segment references are required only to select directions in the RePrice request. Book uses the refreshed itemCodeRef and priceCodeRef (and the existing uniqueTransID envelope field); it does not require response segment references. Null/missing response segment fields are preserved, without synthesizing or copying Search references into the response. Complex carrier/route-specific markup remains subject to the same Search scope guard. Nonzero ancillary coverage and multiple components remain unsupported by the shared projection.

## Selling response and persistence

The supplier item1/item2 envelope is preserved, with shared passenger-level pricing applied once to the **new original supplier fare**. Current applicable markup rules are resolved using the trusted current client audience/agent and refreshed fare. Each successful revision additionally stores `selected_directions`: zero-based directionIndices, publicSegmentCodeRefs, supplierSegmentCodeRefs and the chosen original directions. The Search snapshot remains unchanged. Each successful revision stores the original and selling envelopes, supplier-reference map, winning rule/version, audience/agent, currency and expiration. Rule changes therefore affect the next successful RePrice and are versioned; a stored accepted revision is never repriced in place.

`uniqueTransID` and `itemCodeRef` remain platform Search/offer UUIDs; `priceCodeRef` is a new platform UUID identifying this pricing snapshot. Refreshed supplier references stay in the private original snapshot and reference map. Book uses the selected revision's refreshed transaction, item and price references. It does not send segmentCodeRefs.

Headers:

- `X-Pricing-Version`: increasing integer scoped to the Search offer.
- `X-Price-Acceptance-Required: true`: explicitly accept each returned revision, even when the supplier says `isPriceChanged=false`.

`isPriceChanged` is true when the supplier reports a change or the current selling total differs from the original displayed Search total. Explicit acceptance is required regardless, covering refreshed terms as well. The flag alone is not evidence of tax correctness. Expiration does not extend the original platform offer's 10-minute lifetime and is not a claimed supplier guarantee.

## Acceptance

```http
POST /api/Reprice/accept
Authorization: Bearer <machine token>
Content-Type: application/json
```

```json
{"priceCodeRef":"<platform priceCodeRef from RePrice>"}
```

```json
{"priceCodeRef":"<same UUID>","pricingVersion":1,"accepted":true}
```

This only records local client acceptance of the exact stored selling/terms snapshot; it does not hold inventory, book or issue a ticket. Same-revision retries are idempotent. Only the latest successful unexpired revision can be accepted, and client audience/agent and supplier availability must still match. A new successful RePrice supersedes older revisions for subsequent acceptance; previous acceptance timestamps remain historical evidence. RePrice/acceptance take the same per-offer database lock to serialize version changes.

Future Book must independently require ownership, latest valid accepted revision, current supplier availability, booking/direct-issue permissions and all remaining booking policies. A subsequent hold Book implementation is documented in [BOOKING_API.md](BOOKING_API.md); holds require no payment authorization; supplier/client enablement and accepted-quote checks still apply.

## Errors

| Status | Code / condition |
|---|---|
| 401 / 403 | Missing authentication / missing `search:read` permission |
| 404 | `NOT_FOUND`: unknown or another client's offer/price |
| 409 | `FARE_UNAVAILABLE`: select another offer, optionally on the same airline; no automatic switch |
| 409 | `REPRICE_REQUIRED`: prior quote blocked by a rejected revalidation; obtain a fresh successful RePrice |
| 409 | `NEW_SEARCH_REQUIRED`: disabled supplier or changed availability epoch |
| 409 | `PRICE_VERSION_SUPERSEDED`: acceptance of an older successful revision |
| 409 | `PRICE_CONTEXT_CHANGED`: client audience/agent changed since pricing |
| 410 | `OFFER_EXPIRED` / `PRICE_EXPIRED` / `SUPPLIER_SESSION_EXPIRED`: new Search required |
| 422 | Invalid or mismatched offer/segment/commission references |
| 422 | `PRICING_CONFIGURATION_ERROR`: no applicable current markup rule |
| 422 | `SUPPLIER_PRICING_COVERAGE_UNSUPPORTED`: tax/component/ancillary coverage fails existing pricing validation |
| 422 | Passenger, itinerary or currency mismatch; unsupported branded fare/tax redemption |
| 502 | `SUPPLIER_REPRICE_FAILED`: transport/size failure or supplier business failure; raw supplier messages are not exposed |
| 504 | `SUPPLIER_TIMEOUT` |

A failed RePrice does not create a new usable pricing snapshot. Supplier pricing and itinerary validation remain enforced. Optional RePrice response segment references are not treated as a failure. The existing strict coverage guard remains; this implementation does not settle a tax-normalization or offer-filtering policy.

## Validation and rollout

Migration `0010_reprice_direction_selection.sql` adds the nullable selection record; older revisions remain null rather than having a choice inferred. Apply it before running this build.

Migration `0007_reprice_versions.sql` creates the version/acceptance persistence with client/offer and rule-version foreign keys. Apply migrations before restarting a running server; normal readiness rejects mismatched migration checksums. No existing supplier/client/markup settings are changed by this migration.

Previous bookings do not block fresh RePrice/acceptance for another intentional booking; existing bookings retain their own pricing snapshot.

The local database suite covers markup once, refreshed refs, original snapshots, idempotent acceptance, superseded versions, foreign clients, expiry, disabled/re-enabled connections, tax/currency failures and supplier business errors. The opt-in `tests/live_search.rs` smoke additionally runs public Search → FareRules → RePrice → local acceptance against real adapters in an empty disposable database. It never calls supplier booking or ticketing.


## Fare rejection and selecting another offer

The three observed supplier fare messages (no valid fare, no combinable fare, requested class unavailable) map to HTTP 409 `FARE_UNAVAILABLE`. The observed invalid/expired session message maps to HTTP 410 `SUPPLIER_SESSION_EXPIRED`. Other supplier failures stay generic 502; raw messages are not returned to clients. Exact captured messages are used, not broad substring guesses.

A classified rejection sets the selected offer's `reprice_required` flag (migration 0011). Prior accepted snapshots remain available for audit but cannot be accepted again or used for a new booking until that offer obtains a fresh successful RePrice. A success creates a new revision and clears the flag; older versions remain superseded. The flag is per offer, so a same-airline alternative is independent and must receive its own RePrice and explicit price acceptance. Existing booking retry results are not rewritten.

See [PREBOOKING_FLOW.md](PREBOOKING_FLOW.md) for the customer-selection flow. Price acceptance is local and does not reserve, book or issue, including when `bookable=false`.

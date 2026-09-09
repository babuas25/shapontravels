# RePrice and price acceptance

`POST /api/Reprice` uses a machine token with `search:read`. It reprices one owned, unexpired Search offer through its saved supplier connection. No booking, cancellation or ticketing operation is called.

## Request

```json
{
  "uniqueTransID": "<platform Search UUID>",
  "itemCodeRef": "<platform Search offer UUID>",
  "segmentCodeRefs": ["<platform segment UUIDs from Search, in returned order>"],
  "brandedFareRefs": "",
  "taxRedemptions": [],
  "commissionOnTaxes": []
}
```

Use the original public Search references on each RePrice request. Arbitrary supplier references, foreign offers and modified segment selections are rejected. All returned Search segments must be supplied in order; subset/alternative selection is not implemented. Nonempty branded fares and tax redemption are unsupported. Commission input may be omitted/empty; the server forwards the original supplier commission data. Nonempty caller commission data must equal that saved value.

The connection must still be Search-enabled at the same availability epoch. Its configured currency must match Search and the refreshed response. Availability/expiry are checked again after the upstream read. RePrice enforces passenger type counts, carrier and route endpoints/continuity. Complex carrier/route-specific markup remains subject to the same Search scope guard. Nonzero ancillary coverage and multiple components remain unsupported by the shared projection.

## Selling response and persistence

The supplier item1/item2 envelope is preserved, with shared passenger-level pricing applied once to the **new original supplier fare**. Current applicable markup rules are resolved using the trusted current client audience/agent and refreshed fare. Each successful revision stores the original and selling envelopes, supplier-reference map, winning rule/version, audience/agent, currency and expiration. Rule changes therefore affect the next successful RePrice and are versioned; a stored accepted revision is never repriced in place.

`uniqueTransID` and `itemCodeRef` remain platform Search/offer UUIDs; `priceCodeRef` is a new platform UUID identifying this pricing snapshot. Refreshed supplier references stay in the private original snapshot and reference map. Further Book integration must use the selected revision's refreshed references, never the original Search references.

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
| 409 | `NEW_SEARCH_REQUIRED`: disabled supplier or changed availability epoch |
| 409 | `PRICE_VERSION_SUPERSEDED`: acceptance of an older successful revision |
| 409 | `PRICE_CONTEXT_CHANGED`: client audience/agent changed since pricing |
| 410 | `OFFER_EXPIRED` / `PRICE_EXPIRED` |
| 422 | Invalid or mismatched offer/segment/commission references |
| 422 | `PRICING_CONFIGURATION_ERROR`: no applicable current markup rule |
| 422 | `SUPPLIER_PRICING_COVERAGE_UNSUPPORTED`: tax/component/ancillary coverage fails existing pricing validation |
| 422 | Passenger, itinerary or currency mismatch; unsupported branded fare/tax redemption |
| 502 | `SUPPLIER_REPRICE_FAILED`: transport/size failure or supplier business failure; raw supplier messages are not exposed |
| 504 | `SUPPLIER_TIMEOUT` |

A failed RePrice does not create a new usable pricing snapshot. Known supplier CNN-tax and multicity errors are not silently corrected or bypassed. The existing strict coverage guard remains; this implementation does not settle a tax-normalization or offer-filtering policy.

## Validation and rollout

Migration `0007_reprice_versions.sql` creates the version/acceptance persistence with client/offer and rule-version foreign keys. Apply migrations before restarting a running server; normal readiness rejects mismatched migration checksums. No existing supplier/client/markup settings are changed by this migration.

Previous bookings do not block fresh RePrice/acceptance for another intentional booking; existing bookings retain their own pricing snapshot.

The local database suite covers markup once, refreshed refs, original snapshots, idempotent acceptance, superseded versions, foreign clients, expiry, disabled/re-enabled connections, tax/currency failures and supplier business errors. The opt-in `tests/live_search.rs` smoke additionally runs public Search → FareRules → RePrice → local acceptance against real adapters in an empty disposable database. It never calls supplier booking or ticketing.

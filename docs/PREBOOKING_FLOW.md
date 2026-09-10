# Customer-selected alternatives: Search through price acceptance

This backend flow ends at explicit local price acceptance. It does not automatically call Book, issue tickets or reserve inventory. A human frontend/dashboard remains outside the requirements scope.

1. Call `POST /api/Search` using the customer's route, dates and passenger mix. Retain all returned fare options and their platform references.
2. Customer chooses an offer and exactly one complete direction per route. Use that offer's `uniqueTransID`, `itemCodeRef` and selected `segmentCodeRefs` for FareRules/RePrice. Never combine references from different offers or alternative directions.
3. Optionally retrieve `POST /api/FareRules` for that selection. FareRules now forwards only the chosen directions, consistent with RePrice.
4. Call `POST /api/Reprice`. On `409 FARE_UNAVAILABLE`, show other offers from the same Search with the same `platingCarrier`, excluding the rejected item. Customer may choose another class/fare/flight on that airline. Availability is verified by another RePrice; Search does not guarantee it.
5. RePrice the customer's new selection using its own references and original stored supplier. There is no automatic airline/supplier change and no automatic fallback to another price.
6. Display the successful selling total, passenger breakdown, conditions and `bookable` value. Null RePrice segment refs are valid; do not copy Search refs into the response.
7. After the customer agrees, call `POST /api/Reprice/accept` with the new `priceCodeRef`. Stop here. `accepted:true` records local agreement only and does not mean a booking or ticket exists.

For expired Search/price/supplier-session references or `NEW_SEARCH_REQUIRED`, run a fresh Search. Never update just the transaction/item while retaining stale segment refs. A supplier's exact expiry can be shorter than the platform's ten-minute reference lifetime.

## Request examples

FareRules uses the selected alternate offer:

```json
{
  "uniqueTransID": "<Search UUID>",
  "itemCodeRef": "<customer-selected alternate offer UUID>",
  "segmentCodeRefs": ["<its selected segment UUIDs, in route order>"],
  "brandedFareRefs": ""
}
```

RePrice uses the same selection:

```json
{
  "uniqueTransID": "<Search UUID>",
  "itemCodeRef": "<customer-selected alternate offer UUID>",
  "segmentCodeRefs": ["<its selected segment UUIDs, in route order>"],
  "brandedFareRefs": "",
  "taxRedemptions": [],
  "commissionOnTaxes": []
}
```

Local acceptance uses only the successful new price revision:

```json
{"priceCodeRef":"<successful RePrice priceCodeRef UUID>"}
```

## Error handling

| Response | Client action |
|---|---|
| 409 `FARE_UNAVAILABLE` | Offer another fare/class/flight on the same airline; RePrice the selected alternative |
| 410 `SUPPLIER_SESSION_EXPIRED`, `OFFER_EXPIRED` or `PRICE_EXPIRED` | New Search and selection with all new references |
| 409 `NEW_SEARCH_REQUIRED` | New Search under current supplier configuration |
| 409 `REPRICE_REQUIRED` | Previously priced offer had a rejected revalidation; fresh successful RePrice required |
| 409 `PRICE_VERSION_SUPERSEDED` | Review and accept the latest successful price version |
| 422 `OFFER_REFERENCE_MISMATCH` | Correct the selection; do not mix offers/directions |
| 502 `SUPPLIER_REPRICE_FAILED` | Generic supplier failure; do not assume it means every fare is unavailable |

A rejection does not create a new usable pricing revision. Classified fare/session rejections block the selected offer's old quote via a persisted flag; another same-airline offer remains independently usable. Exact known supplier messages are mapped; unknown messages retain the generic error.

## Instant purchase boundary

`bookable=false` may mean a Book call would issue immediately. Search, FareRules, RePrice and local acceptance do not call Book and remain usable to inspect such fares. Acceptance does not grant issue intent or execution permission. Existing Book code rejects direct issue and requires both Search and RePrice `bookable=true` for holds. No Book/NewTicket endpoint is invoked by this flow.

Apply migration `0011_reprice_required.sql` before running the new build. Tests and live evidence are recorded in REQUIREMENTS.md; this change does not complete the separate pending canonical matching, complex markup, aggregate summaries, cancellation or ticketing requirements.


FareRules supplier business/transport failures return HTTP 502 `{"error":"UPSTREAM_FARE_RULES_ERROR"}` without exposing raw supplier messages. The request deadline still returns 504 `SUPPLIER_TIMEOUT`. Show “Fare rules are currently unavailable” and allow customer-initiated RePrice for the selected offer; this error does not mark the fare unavailable or invalidate its price. Do not invent cancellation/refund rules. RePrice success still requires explicit local acceptance, and this flow stops before Book/Issue.

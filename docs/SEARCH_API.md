# Public Search and FareRules — initial direct-flight release

## Setup in Swagger

1. Authorize an admin session. Create and activate an applicable markup rule via `/admin/markup-rules` (e.g. B2B, all airlines/all routes, fixed `"500"`, BDT).
2. Use GET `/admin/suppliers` then PUT `/admin/suppliers/{id}` to enable `search_enabled` for the suppliers you want, retaining current settings and passing `expected_version`. Booking/ticketing gates need not be enabled.
3. Create or use an active API client with `search:read`; exchange Client ID/Secret at `/auth/token`, then authorize `machine_token`.
4. POST `/api/Search` using its example and a future departure date. No passenger names, passports, payment or ticket issue are involved.

```json
{
  "routes": [{"origin": "DAC", "destination": "CXB", "departureDate": "2026-09-29"}],
  "adults": 1,
  "childs": 0,
  "infants": 0,
  "cabinClass": 1,
  "preferredCarriers": [],
  "prohibitedCarriers": [],
  "childrenAges": []
}
```

Search accepts documented cabin classes 1–5. The initial request bounds are 1–9 seated passengers, at least one adult, at most one infant per adult, and at most six requested routes. Child ages must match the child count and lie in 2–11. Only the observed optional fareType `1` is supported until more supplier enum evidence exists. Unknown fields such as client-supplied agent/audience overrides are rejected.

## Response and pricing

Results are at `item1.airSearchResponses[]`. Each offer's `totalPrice` is the count-aggregated selling total after the applicable active rule and approved per-passenger two-decimal half-up rounding. Passenger/component discount values derive from the rounded selling totals. Original pricing snapshots and rule IDs/versions remain private in PostgreSQL.

Search summary/filter metadata is aggregated **after supplier selection and markup**, from the final `item1.airSearchResponses` only. `X-Search-Summary-Scope: retained-selling-offers` identifies this contract. Aggregation is separate from markup; the offer projection still changes only its permitted pricing fields.

| Existing field | Aggregate meaning |
| --- | --- |
| `totalFlights` | Number of returned fare offers, including distinct fare classes; not unique physical flights or direction combinations. |
| `supplierCount` | Number of supplier connections represented by retained offers; excludes failed, disabled and entirely discarded sources. |
| `minMaxPrice.minNetPrice` / `maxNetPrice` | Minimum/maximum final group selling `totalPrice`; AIT is already included. |
| `minNetPriceAit` / `maxNetPriceAit` | Count-aggregated AIT of the corresponding net-price extremum offer. Do not add it again. Tied minima use the first retained offer; tied maxima use the last. |
| `minMaxPrice.minPrice` / `maxPrice` | Independent extrema of gross Base+Tax+AIT, aggregated by passenger count; gross is not the payable selling price and receives no markup. |
| `airlineFilters[]` | One row per retained `platingCarrier`, sorted by carrier code. Counts and minimum net/gross fields use only that airline's retained fares. |
| `stops` | Sorted distinct supplier-reported direction stop counts across every route and selectable direction. Counts are not summed across a roundtrip/multicity itinerary. |
| `totalPages` | If numeric, 1 for a nonempty complete response or 0 for empty. Existing null stays null. No public pagination endpoint is provided. |

For a successful empty Search, numeric summary/count fields become zero and existing airline/stops arrays become empty. Missing keys and null values remain absent/null: unavailable metadata is not fabricated. Airline rows use the first matching source row in stable supplier-ID order to preserve its field names and unknown values; rows for absent airlines are removed. Unknown envelope/summary/row fields, currency and request-time metadata remain unchanged and are not claimed to be aggregated. The existing nonempty pagination token is rebound to the platform search ID; it does not provide a next-page capability.

Known populated summary fields with invalid types, invalid required offer summary inputs or a retained airline without any source filter-row template fail with HTTP 422 `SUPPLIER_SUMMARY_UNSUPPORTED`. This rolls back the Search/offer inserts, rather than returning stale summary values. Supplier diagnostic messages and upstream account IDs in status entries are sanitized separately.

Headers:

- `X-Search-Currency`: trusted configured currency, currently BDT for all three accounts, explicitly confirmed by the user. Search's item1 currency is null in the observed samples; offer-level currency is absent. No nonnull currency field is invented.
- `X-Search-Partial`: true if an active connection failed/timed out; intentionally disabled connections are not counted. All active connection calls start concurrently from a database snapshot.
- `X-Search-Summary-Scope`: `retained-selling-offers`, as defined above.

Only returned offers with matching passenger counts, reconciled single-component pricing and no unverified ancillary/service charge are projected. Nonempty branded-fare mapping is deferred. Missing markup configuration fails with `PRICING_CONFIGURATION_ERROR`; unsupported mapping/coverage fails rather than exposing a partially marked-up offer.

**Supplier selection precedes markup.** For conservatively equivalent offers, Search chooses the lowest original supplier `totalPrice` across the active connections, using exact decimal comparison. This is the total for all passengers, excluding platform markup. Equal supplier totals prefer **Takeoff → Firsttrip → Triplover**, as approved on 2026-09-09. Each retained class/fare option then receives its applicable markup; the public `totalPrice` remains the selling total. Even if passenger rounding makes the chosen selling total higher, supplier-total ranking is preserved.

Equivalence is deliberately strict: reported `bookingClass`/RBD (for example Q or V), `serviceClass`, fare basis, carriers, ordered routes/segments, dates/times, baggage and refundability must match. `cabinClass` is optional and is excluded from the main key: null/missing/empty labels can match a known label when all required attributes agree. Explicitly conflicting nonempty cabin labels on any corresponding segment keep the entire otherwise-matching group separate, so an unknown label cannot bridge Economy and Business. Unknown fields, base/tax/AIT breakdown, fee metadata and other non-reference attributes remain in the comparison. Only the evidenced source/transaction/item/segment/component references and quoted total/discount fields are removed from the private key; original and returned offer shapes are preserved. Different display metadata, baggage representations or base/tax breakdowns may therefore keep otherwise similar offers separate pending verified normalization. Identical offers from the same supplier tied at its lowest price retain their original order.

Missing required attributes, codeshares and ambiguous direction alternatives remain separate. The captured Triplover domestic `bookingClass: "Q"` with `cabinClass: null` can now participate in matching. No cabin value is inferred or rewritten; the original null/missing shape is preserved. Q and V are separate RBD options even when both lack cabin labels. Complete canonical equivalence remains unfinished. Existing passenger/currency/component validation runs before selection for every source offer: an invalid losing offer still fails the Search under the existing strict policy. Only selected original offers and their matching source references are persisted; subsequent operations continue on that supplier.

Specific carrier/route matching currently accepts only one route with one direct segment and consistent plating/segment carrier, with no code-share indication. User-approved All/All exception: complex/codeshare offers can be priced when every active rule in the client audience/agent and currency candidate set has no airline or route restriction. Normal agent/B2B fallback priority is preserved. If any candidate has an airline or route restriction, ambiguous offers still return `COMPLEX_SCOPE_MATCHING_UNRESOLVED`; no first-leg shortcut is used. Branded-fare and component-coverage checks still apply.

## References and FareRules

Search replaces supplier transaction/item/segment references with platform UUIDs. Original references, supplier connection, owner, rule version and availability epoch are stored privately. Platform references expire after ten minutes; this is a local retention/use limit, not a guarantee of supplier TTL. Supplier references can still expire earlier.

To call POST `/api/FareRules`, take the selected returned offer's `uniqueTransID`, `itemCodeRef`, and the ordered `segmentCodeRef` values from exactly one complete selected direction per route:

```json
{
  "uniqueTransID": "returned-search-uuid",
  "itemCodeRef": "returned-offer-uuid",
  "segmentCodeRefs": ["returned-segment-uuid"],
  "brandedFareRefs": ""
}
```

The server checks ownership, expiry and an exact complete-direction match, then uses the stored original references on the original supplier. Foreign offers return 404; expired references 410; tampered reference sets 422. An admin token cannot call these commercial routes. FareRules references returned by suppliers are rebound to the client's platform context. FareRules does not book, accept a price or issue a ticket.

Platform auth/configuration/validation errors currently retain the established platform error format `{"error":"CODE"}`; success pipeline responses use supplier item1/item2 envelopes. Full flight error/OpenAPI schema compatibility remains part of final contract work.

## Limits and validation

- Public RePrice and local price acceptance are documented in [REPRICE_API.md](REPRICE_API.md). Hold Book/status/reconciliation are documented in [BOOKING_API.md](BOOKING_API.md); hold booking uses the approved no-payment policy; Cancel, NewTicket and ticket issue remain unavailable.
- Search response bodies have a 64 MiB cap; other supplier reads retain an 8 MiB cap. Connection timeouts come from admin configuration; the supplier adapter and whole request remain bounded.
- Complete canonical equivalence, dynamic complex-route matching, branded fares and multiple components remain unfinished. Search and offer expiry indexes are added; retention cleanup scheduling remains deployment work.
- The working database's clients/rules/supplier activation settings are not changed by integration tests. Explicit production smoke uses a separate empty local database with temporary test identities/default rule.

Tests cover all seven active supplier subsets, no-active/no-rule outcomes, partial/all failure, exact markup, reference rebinding and persistence, FareRules same-supplier routing, ownership/tampering/expiry, and machine/admin isolation. A production read-only public-flow smoke returned 78 offers with `X-Search-Partial: false`, verified the fixed-500 projection and successfully retrieved FareRules. No supplier mutations were sent.


## Choosing another fare on the same airline

A rejected RePrice applies to the selected offer, not every offer from its airline. Keep the Search result and filter other entries by the same `platingCarrier` (e.g. `6E`), excluding the failed `itemCodeRef`. Distinct RBD, fare and flight options remain selectable. This is a plating-carrier match, not a guarantee that all segments are operated by that airline. Customer selection determines the next offer; the backend does not silently change the item or supplier.

Use the other offer's own item/selected segment references for FareRules and RePrice. If the Search or supplier session expired, run a new Search and replace all references together. See [prebooking flow](PREBOOKING_FLOW.md).


FareRules supplier business/transport failures return HTTP 502 `{"error":"UPSTREAM_FARE_RULES_ERROR"}` without exposing raw supplier messages. The request deadline still returns 504 `SUPPLIER_TIMEOUT`. Show “Fare rules are currently unavailable” and allow customer-initiated RePrice for the selected offer; this error does not mark the fare unavailable or invalidate its price. Do not invent cancellation/refund rules. RePrice success still requires explicit local acceptance, and this flow stops before Book/Issue.


## Processing and load measurement

Large supplier inventories are moved rather than copied between response envelopes. Retained offers persist in batches of at most 64 rows inside one transaction; a later insert failure rolls back all batches. Offer data, selection, pricing and response shape are unchanged. Non-sensitive `search_performance` logs contain source/returned counts and supplier/preparation/persistence/total phase durations; persistence includes pricing/summary/JSON binding as well as database execution.

See [the measured comparison](evidence/SEARCH_PERFORMANCE_2026-09-10.md) for one/four-concurrent offline replays. These are local process measurements, not a production VPS capacity guarantee. Negotiated response compression is described below; pagination, admission limits and expired-row cleanup remain separate work.


## Lossless response compression

Clients may send `Accept-Encoding: gzip`. Eligible responses of at least 1,024 bytes use `Content-Encoding: gzip` and `Vary: accept-encoding`; compressed responses do not retain the original Content-Length. The application uses tower-http gzip with `CompressionLevel::Fastest` to limit CPU cost. Default content-type exclusions remain enabled.

Clients omitting Accept-Encoding, requesting identity, or rejecting gzip with `gzip;q=0` receive the original representation. Gzip decoding recovers the original JSON bytes: every retained offer, unknown field, null/missing distinction, exact numeric lexeme and reference remains intact. Existing Search summary/partial headers, authentication, no-store and request IDs remain in effect. Small health/error responses stay uncompressed.

Compression reduces transferred bytes; it does not reduce decoded JSON size, database snapshots or the work of preparing offers. Current request/phase logs finish before response-body transmission/compression is fully polled; use full-body HTTP timings for that cost. The local replay example accepts `LOAD_ACCEPT_ENCODING=identity|gzip` and reports wire bytes, decoded bytes and decode time separately. See [gzip measurements](evidence/SEARCH_GZIP_2026-09-10.md).


### Borrowed coverage validation and equivalence keys

Source-offer coverage validation now reads the original offer without constructing a discarded selling snapshot. It uses the same calculation and optional-field checks as projection; every source offer is still validated before selection. Comparison keys serialize a borrowed view with exactly the prior field exclusions and bytes, preserving unknown fields at their original scope. Supplier winner and cabin-conflict rules remain unchanged.

[Final replay evidence](evidence/SEARCH_BORROWED_VALIDATION_2026-09-10.md) shows lower preparation CPU, with mixed full-body latency and no measured peak-RAM reduction. Selling snapshots, serialized comparison keys and persisted inventory remain allocated; broader memory optimization is still required.

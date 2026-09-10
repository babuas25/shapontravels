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

Markup scope uses the **first requested route** and the **first flight segment's `airlineCode` in that route**, for one-way, connecting, return and multicity fares. `platingCarrier`, later segment airlines and codeshare flags do not choose the markup airline. For DAC→SIN via KUL with MH first and SQ second, the scope is MH + DAC→SIN. Every route alternative must match requested endpoints and continuous segments. When alternatives in the first route have different or missing/invalid first airlines, airline-specific candidates remain guarded by `COMPLEX_SCOPE_MATCHING_UNRESOLVED`; airline-independent rules can still use the validated route. No arbitrary alternative is selected or removed.

For DAC→SIN, SIN→BKK, BKK→DAC, only DAC→SIN selects the route rule; later-route rules are not added, averaged or used as the first available match. Existing Agent/B2B/B2C and scope priority/fallback remain unchanged; no applicable rule returns `PRICING_CONFIGURATION_ERROR`. Reverse routes remain directional. Current whole-journey passenger fares receive one markup, followed by count aggregation. Actual separate leg/passenger prices still require verified mapping before per-leg calculation. Branded and component coverage guards remain. The historical complex All/All exception is preserved when all candidates are airline/route independent.

## References and FareRules

Search replaces supplier transaction/item/segment references with platform UUIDs. Original references, supplier connection, owner, rule version and availability epoch are stored privately. Platform references expire after ten minutes; this is a local use limit, not a guarantee of supplier TTL. Supplier references can still expire earlier.

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
- Complete canonical equivalence, dynamic complex-route matching, branded fares and multiple components remain unfinished. Search and offer expiry indexes are present; temporary Search cleanup is described below.
- The working database's clients/rules/supplier activation settings are not changed by integration tests. Explicit production smoke uses a separate empty local database with temporary test identities/default rule.

Tests cover all seven active supplier subsets, no-active/no-rule outcomes, partial/all failure, exact markup, reference rebinding and persistence, FareRules same-supplier routing, ownership/tampering/expiry, and machine/admin isolation. A production read-only public-flow smoke returned 78 offers with `X-Search-Partial: false`, verified the fixed-500 projection and successfully retrieved FareRules. No supplier mutations were sent.


## Choosing another fare on the same airline

A rejected RePrice applies to the selected offer, not every offer from its airline. Keep the Search result and filter other entries by the same `platingCarrier` (e.g. `6E`), excluding the failed `itemCodeRef`. Distinct RBD, fare and flight options remain selectable. This is a plating-carrier match, not a guarantee that all segments are operated by that airline. Customer selection determines the next offer; the backend does not silently change the item or supplier.

Use the other offer's own item/selected segment references for FareRules and RePrice. If the Search or supplier session expired, run a new Search and replace all references together. See [prebooking flow](PREBOOKING_FLOW.md).


FareRules supplier business/transport failures return HTTP 502 `{"error":"UPSTREAM_FARE_RULES_ERROR"}` without exposing raw supplier messages. The request deadline still returns 504 `SUPPLIER_TIMEOUT`. Show “Fare rules are currently unavailable” and allow customer-initiated RePrice for the selected offer; this error does not mark the fare unavailable or invalidate its price. Do not invent cancellation/refund rules. RePrice success still requires explicit local acceptance, and this flow stops before Book/Issue.


## Processing and load measurement

Large supplier inventories are moved rather than copied between response envelopes. Retained offers persist in batches of at most 64 rows inside one transaction; a later insert failure rolls back all batches. Offer data, selection, pricing and response shape are unchanged. Non-sensitive `search_performance` logs contain source/returned counts and supplier/preparation/persistence/total phase durations; persistence includes pricing/summary/JSON binding as well as database execution.

See [the measured comparison](evidence/SEARCH_PERFORMANCE_2026-09-10.md) for one/four-concurrent offline replays. These are local process measurements, not a production VPS capacity guarantee. Negotiated response compression is described below; pagination remains separate work; Search admission is described below; temporary Search cleanup is described below.


## Lossless response compression

Clients may send `Accept-Encoding: gzip`. Eligible responses of at least 1,024 bytes use `Content-Encoding: gzip` and `Vary: accept-encoding`; compressed responses do not retain the original Content-Length. The application uses tower-http gzip with `CompressionLevel::Fastest` to limit CPU cost. Default content-type exclusions remain enabled.

Clients omitting Accept-Encoding, requesting identity, or rejecting gzip with `gzip;q=0` receive the original representation. Gzip decoding recovers the original JSON bytes: every retained offer, unknown field, null/missing distinction, exact numeric lexeme and reference remains intact. Existing Search summary/partial headers, authentication, no-store and request IDs remain in effect. Small health/error responses stay uncompressed.

Compression reduces transferred bytes; it does not reduce decoded JSON size, database snapshots or the work of preparing offers. Current request/phase logs finish before response-body transmission/compression is fully polled; use full-body HTTP timings for that cost. The local replay example accepts `LOAD_ACCEPT_ENCODING=identity|gzip` and reports wire bytes, decoded bytes and decode time separately. See [gzip measurements](evidence/SEARCH_GZIP_2026-09-10.md).


### Borrowed coverage validation and equivalence keys

Source-offer coverage validation now reads the original offer without constructing a discarded selling snapshot. It uses the same calculation and optional-field checks as projection; every source offer is still validated before selection. Comparison keys serialize a borrowed view with exactly the prior field exclusions and bytes, preserving unknown fields at their original scope. Supplier winner and cabin-conflict rules remain unchanged.

[Final replay evidence](evidence/SEARCH_BORROWED_VALIDATION_2026-09-10.md) shows lower preparation CPU, with mixed full-body latency and no measured peak-RAM reduction. Selling snapshots, serialized comparison keys and persisted inventory remain allocated; broader memory optimization is still required.


### Bounded snapshot lifetimes

Search encodes original JSON in batches of at most 64 offers, then reuses each owned parsed offer for selling prices/references. Each batch persists before its original JSON buffers are released. All batches and final summary validation share one transaction; summary or SQL errors roll back the entire Search. This reduces concurrent original/selling tree allocation without dropping offers or changing pricing, reference validity or persisted data.

The offline example accepts `LOAD_PROFILE_MEMORY=1` for process RSS samples at debug phase markers; normal production logging does not collect memory. [Local memory verification](evidence/SEARCH_MEMORY_LIFETIMES_2026-09-10.md) records measured reductions and limits. Retention cleanup is described below.


## Temporary Search cleanup

The serving process checks every 30 seconds for offers that are at least 15 minutes old, have expired, and belong to an expired Search. Only offers with **no RePrice and no booking** are deleted. All linked offers, RePrice versions, booking states and their required Search headers remain intact; those business records need a separate retention policy. Current Search usability is still 10 minutes.

Cleanup processes at most 512 rows per table in a transaction, uses row locks with SKIP LOCKED, and coordinates instances with an advisory transaction lock. Each tick runs at most 32 batches and checks a five-second budget between batches; statements have a two-second timeout and lock waits a 200 ms timeout. Eligible records are normally removed on a following tick; load, locks or backlog can delay physical deletion. The job does not run in migration/bootstrap modes or when a test merely constructs the router.

Deleted offer payloads are replaced atomically with a small owner-scoped ID marker, retained for 24 hours after deletion. FareRules/RePrice return 410 OFFER_EXPIRED to that owner while the marker is valid; other clients receive 404. After the marker expires, the reference returns 404. No fares, routes, passenger data or reference maps are stored in the marker. Empty expired Search headers are deleted in the same cleanup transaction; a header with any retained offer remains.

Database DELETE makes space reusable through normal PostgreSQL vacuuming; it does not promise that the database file immediately shrinks. Cleanup affects stored temporary data, separately from the in-flight memory improvements. Counts and safe failures are logged without payloads. See [cleanup verification](evidence/SEARCH_CLEANUP_2026-09-10.md).


## Bounded Search admission

Search now acquires process-local capacity **after** machine authentication, scope checks and request validation, but **before** reading current supplier/pricing configuration or making supplier calls. Existing authentication/rate-limit database work occurs before this gate. No connection or transaction is held while waiting for Search capacity.

| Environment variable | Default | Allowed range |
| --- | ---: | ---: |
| `SEARCH_MAX_ACTIVE` | 4 | 1–8 |
| `SEARCH_MAX_QUEUED` | 8 | 0–32 |
| `SEARCH_QUEUE_WAIT_MS` | 2000 | 1–2000 |

A full admission pool or elapsed queue wait returns HTTP **503**, `{"error":"SEARCH_BUSY"}`, and `Retry-After: 1`, retaining the normal no-store/request-ID headers. Retry-After is a suggested delay, not a guarantee of capacity; clients should use bounded backoff with jitter. Existing credential/permission/invalid-request errors take precedence. Busy attempts consume the existing authentication rate-limit allowance, but perform no supplier Search or inventory insert. Existing rate-limit 429 and supplier-failure 503 semantics remain separate.

An admitted Search retains all baseline valid offers, unchanged pricing/reference semantics and atomic persistence. It holds capacity during supplier processing, serialization and consumption of the compressed or identity response body. Completion, body error, disconnect/drop and cancelled waiting/work futures release capacity. The outer body wrapper sits outside compression; returning headers alone does not free the slot. Bytes already handed to HTTP/socket buffers are outside this gate's accounting.

Other endpoints, including health, FareRules, RePrice and booking, do not acquire this gate. Router clones share one admission pool; separate processes/replicas have independent pools. Slow clients can hold active slots, so excess requests may receive SEARCH_BUSY; queue wait is not a total Search/transport deadline. Limits bound the number of expensive responses, not an absolute memory byte budget. These defaults are provisional guardrails, not a measured throughput promise.

The offline replay accepts `LOAD_MAX_ACTIVE`, `LOAD_MAX_QUEUED` and `LOAD_QUEUE_WAIT_MS` with the same bounds, in addition to `LOAD_CONCURRENCY`. Defaults match the application; set and report an explicit queue budget for scheduling comparisons, and do not confuse that experiment with the configured overload policy.

See [admission verification and local RAM/time tradeoff](evidence/SEARCH_ADMISSION_2026-09-10.md).


### Bursts of 10-12 customers

The updated default has four active slots and eight queued slots: up to twelve simultaneous Search requests can be admitted/waiting, assuming each customer sends one request and the pool was initially empty. Queue wait is bounded at two seconds, as explicitly requested by the user. Values above 2000 ms are rejected at startup. Later arrivals or requests that wait longer receive SEARCH_BUSY; this is not a promise that every supplier response completes in time. The bound counts requests, not unique customers.

The existing VPS proxy timeout was read-only verified as 150 seconds. The earlier 240-second timeout proposal for a 90-second queue is withdrawn; this two-second policy requires no queue-driven proxy timeout increase. No live Nginx change has been made.

See [twelve-arrival comparison](evidence/SEARCH_BURST_2026-09-10.md) for local four-versus-six active measurements. The earlier two-active default was superseded after the user specified simultaneous bursts of 10-12 customers. Historical all-success replays used longer queue budgets; they do not prove all twelve arrivals succeed with the final two-second limit. Controlled VPS validation of this twelve-arrival policy is now recorded below; uncapped production capacity remains unproven.

For deliberate overload measurement only, the offline replay accepts `LOAD_ALLOW_BUSY=1`: it records verified SEARCH_BUSY responses instead of aborting, while other failures still fail the run. Normal replay remains all-success by default.

Final two-second local burst replay returned eight successes and four SEARCH_BUSY responses per twelve-arrival wave in each of three runs. All successful offers were preserved. This is a measured local outcome, not a guarantee or prediction for live supplier traffic.


The controlled VPS replay of the final policy returned four successes and eight SEARCH_BUSY responses per twelve-arrival wave in all three runs. Median busy full-body latency was 3.099 seconds, so the two-second admission timer must not be presented as a two-second end-to-end response guarantee. See [VPS burst evidence](evidence/SEARCH_VPS_BURST_2026-09-10.md) for CPU/RAM caps, timing limits and complete outcomes. Production has not been updated.

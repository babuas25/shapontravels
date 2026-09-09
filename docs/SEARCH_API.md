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

Existing item1 summary/filter fields are retained from the first successful supplier for shape compatibility. **They are not aggregated selling-price summaries.** In particular do not display `minMaxPrice`, `totalFlights`, pagination/filter metadata as this platform's aggregate selling totals; derive UI prices/counts from returned offers. `X-Search-Summary-Scope: first-successful-supplier` explicitly marks this initial limitation. A full aggregate summary contract remains pending. Unknown offer fields are preserved; markup itself changes only permitted pricing values. Supplier diagnostic messages and upstream account IDs in status entries are sanitized separately from markup.

Headers:

- `X-Search-Currency`: trusted configured currency, currently BDT for all three accounts, explicitly confirmed by the user. Search's item1 currency is null in the observed samples; offer-level currency is absent. No nonnull currency field is invented.
- `X-Search-Partial`: true if an active connection failed/timed out; intentionally disabled connections are not counted. All active connection calls start concurrently from a database snapshot.
- `X-Search-Summary-Scope`: limitation described above.

Only returned offers with matching passenger counts, reconciled single-component pricing and no unverified ancillary/service charge are projected. Nonempty branded-fare mapping is deferred. Missing markup configuration fails with `PRICING_CONFIGURATION_ERROR`; unsupported mapping/coverage fails rather than exposing a partially marked-up offer.

**Fare equivalence/lowest selection is not complete.** Unknown equivalence remains separate, including cross-supplier duplicates. All distinct fare options are retained; this release does not claim that it has collapsed equivalent options to their cheapest supplier. Stable supplier ordering is presentation ordering, not an approved equal-price selection rule.

Specific carrier/route matching currently accepts only one route with one direct segment and consistent plating/segment carrier, with no code-share indication. User-approved All/All exception: complex/codeshare offers can be priced when every active rule in the client audience/agent and currency candidate set has no airline or route restriction. Normal agent/B2B fallback priority is preserved. If any candidate has an airline or route restriction, ambiguous offers still return `COMPLEX_SCOPE_MATCHING_UNRESOLVED`; no first-leg shortcut is used. Branded-fare and component-coverage checks still apply.

## References and FareRules

Search replaces supplier transaction/item/segment references with platform UUIDs. Original references, supplier connection, owner, rule version and availability epoch are stored privately. Platform references expire after ten minutes; this is a local retention/use limit, not a guarantee of supplier TTL. Supplier references can still expire earlier.

To call POST `/api/FareRules`, take the selected returned offer's `uniqueTransID`, `itemCodeRef`, and every `segmentCodeRef` in route/segment order:

```json
{
  "uniqueTransID": "returned-search-uuid",
  "itemCodeRef": "returned-offer-uuid",
  "segmentCodeRefs": ["returned-segment-uuid"],
  "brandedFareRefs": ""
}
```

The server checks ownership, expiry and exact segment reference match, then uses the stored original references on the original supplier. Foreign offers return 404; expired references 410; tampered reference sets 422. An admin token cannot call these commercial routes. FareRules references returned by suppliers are rebound to the client's platform context. FareRules does not book, accept a price or issue a ticket.

Platform auth/configuration/validation errors currently retain the established platform error format `{"error":"CODE"}`; success pipeline responses use supplier item1/item2 envelopes. Full flight error/OpenAPI schema compatibility remains part of final contract work.

## Limits and validation

- Public RePrice and local price acceptance are documented in [REPRICE_API.md](REPRICE_API.md). Hold Book/status/reconciliation are documented in [BOOKING_API.md](BOOKING_API.md); hold booking uses the approved no-payment policy; Cancel, NewTicket and ticket issue remain unavailable.
- Search response bodies have a 64 MiB cap; other supplier reads retain an 8 MiB cap. Connection timeouts come from admin configuration; the supplier adapter and whole request remain bounded.
- Search summary aggregation, complete canonical equivalence, dynamic complex-route matching, branded fares and multiple components remain unfinished. Search and offer expiry indexes are added; retention cleanup scheduling remains deployment work.
- The working database's clients/rules/supplier activation settings are not changed by integration tests. Explicit production smoke uses a separate empty local database with temporary test identities/default rule.

Tests cover all seven active supplier subsets, no-active/no-rule outcomes, partial/all failure, exact markup, reference rebinding and persistence, FareRules same-supplier routing, ownership/tampering/expiry, and machine/admin isolation. A production read-only public-flow smoke returned 78 offers with `X-Search-Partial: false`, verified the fixed-500 projection and successfully retrieved FareRules. No supplier mutations were sent.

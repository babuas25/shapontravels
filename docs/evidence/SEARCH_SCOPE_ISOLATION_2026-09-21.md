# Search markup-scope isolation — 2026-09-21

## Production diagnosis

Read-only production usage records show ten DAC–SIN searches on 2026-09-21,
16:23–16:34 Asia/Dhaka, failing with `COMPLEX_SCOPE_MATCHING_UNRESOLVED`.
These searches used two or three suppliers; every dispatched supplier recorded
a successful response. They were not supplier transport timeouts. Earlier
multi-supplier searches succeeded. An airline/route-specific SQ SIN–DAC markup
rule had been created at 16:11; the matching code rejects unresolved journeys
whenever relevant audience/currency candidates contain scoped rules.

A fresh read-only supplier Search for DAC–SIN, 2026-09-28, one adult returned
204 TripLover offers. Observed unsupported geometry included:

- DAC–KMG–SHA, followed by PVG–SIN: an airport transfer without continuous
  flight-segment endpoints.
- DAC–KUL, followed by SZB–XSP: an airport transfer and a destination different
  from the requested SIN airport.

The probe counted four continuity failures and three endpoint failures; these
categories can overlap. FirstTrip and TakeOff returned HTTP errors during this
separate probe, so it does not establish their current inventory or availability.
The earlier platform usage records establish successful supplier responses for
the reported failed searches. No switch, markup rule, service or database setting
was changed, and no booking, ticket or notification was created.

## Correction prepared on development

Search and RePrice use the same exact-airport / configured-group resolver. All
133 groups (275 distinct airport codes) are enabled, as requested by the owner,
including single-airport groups. There is no three-group review allowlist.
The observed transfers and alternate arrival airport above now pass geography
validation instead of being excluded. Missing optional direction labels use the
actual segments; missing segment airports are not guessed. Stop count does not
change markup scope.

Markup still uses the first requested route and its verified first airline.
For requested DAC–SIN with actual arrival XSP, the DAC–SIN rule still applies.
This does not add country-level markup rules or change rule priority. A shared
country is never enough to equate different cities. Supplier source data and
booking references remain intact. RePrice still rejects any airport substitution
against the exact selected flight, even within a configured group.

An entire offer with genuinely unresolved geography/airline scope is excluded
before supplier selection, so it cannot suppress verified companion offers.
Original source indices preserve supplier identity and references after filtering.
Partial headers and summary describe retained inventory; all-unverified inventory
returns the existing 422 without persistence. Passenger, monetary, currency,
branded-fare and reference guards apply to every source offer. The historical
all-independent-rule exception remains. There is no promise that arbitrary or
missing location data can be priced safely.

The frontend shows actual segment endpoints (XSP remains XSP), plus airport-change
notices in collapsed results and expanded itinerary details. The mapping does not
assert that a ground transfer is supplied, protected or feasible within a layover.
No schema migration or environment change is required.

## Catalog audit and maintenance

The original frontend `airports.json` and `city-airport-groups.json` are unchanged.
Both were copied byte-for-byte into backend `data/` on 2026-09-21. The backend
`city-airport-groups.json` is now the pricing geography authority. All groups are
exported without requiring a review record. Frontend changes do not update or
override the backend files.

The audit found 6,725 rows, 6,658 active three-letter rows, 6,572 distinct active
codes, 81 duplicate codes and 68 conflicting city/country identities. TTN overlaps
two groups; FBU/NIC are missing active raw airport records. These and raw country/
type mismatches are reported without removing configured memberships. Historical
reviews for three groups are retained as provenance only; stale reviews are
reported. This is not a claim of IATA-certified or fully verified geography.

The resolver preserves direct membership without merging groups: JFK/TTN and
TTN/PHL both match, while JFK/PHL do not. Country alone is insufficient. Invalid
codes, empty/malformed groups, duplicate members within a group and contradictory
country assignments across groups fail generation. Real-world mistakes in group
membership must be corrected in the authoritative group file.

Full audit and input fingerprints:
[AIRPORT_MAPPING_AUDIT_2026-09-21.json](AIRPORT_MAPPING_AUDIT_2026-09-21.json).
After editing backend data, regenerate and verify:

```sh
python3 scripts/audit-airport-mapping.py
python3 scripts/audit-airport-mapping.py --check
python3 scripts/test-airport-mapping.py
```

`--check` detects stale generated files. Maintenance, builds and runtime require
no frontend checkout. The generated group index is loaded once, not per request.
See [catalog instructions](../../data/README.md).

## Verification

- Full disposable PostgreSQL migration/workflow suite passed for the expanded catalog (47.42 seconds).
  All seven enabled supplier subsets exercise eight valid shapes: direct, SHA/PVG,
  KUL/SZB→XSP, BKK/DMK, IST/SAW, LHR/LGW, JFK/TTN and TTN/PHL. Assertions cover scoped rule ID, gross 4349.00, commission
  68.97, payable 4280.03, supplier priority, preserved source/reference mapping,
  FareRules and unchanged-price RePrice. Same-city airport substitution on
  RePrice is rejected. Different-city endpoints/gaps and ambiguous airlines
  remain excluded; money/reference errors still fail. Empty, partial and
  all-unverified cases are covered.
- Rust library: 111 passed, two existing opt-in tests ignored. Resolver tests
  include missing optional labels, conflicting labels, unknown locations,
  same-country different-group rejection, direct overlap without transitive
  matching, and every airport pair in every configured source group.
- All-target Clippy with warnings denied, Rust formatting and both repository diff checks passed.
- Six Python audit regressions and deterministic catalog `--check` passed.
  Both checks are included in CI to catch stale generated mappings.
- Frontend canonical prebooking regressions, TypeScript and changed-file ESLint
  passed, including actual endpoint display without source mutation.
- Browser verification against the synthetic loopback fixture confirmed XSP on
  the card, “Airport change: KUL → SZB” while collapsed, and the transfer-arrangement
  notice in expanded details. No real supplier call, hold or booking was made.

Local only: no commit, push, deployment, migration or live supplier-control change.
Production deployment and post-deployment search verification remain pending.

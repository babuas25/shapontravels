# Supplier search diagnosis and priority correction — 2026-09-21

## Read-only production findings

The production database on `160.25.226.236` reports:

| Supplier | Search enabled | Timeout | Connection version |
| --- | --- | --- | --- |
| FirstTrip | false | 20 seconds | 1 |
| TakeOff | false | 20 seconds | 1 |
| TripLover | true | 60 seconds | 3 |

All three suppliers have configured live endpoints, credentials and BDT currency.
Each authenticated successfully during a direct, read-only Login/Search probe.
No credentials, tokens or supplier booking references are included here.

Probe: DAC–SIN, 2026-09-29, one adult, Economy, no carrier filter. The screenshot
does not show passenger counts; one adult was the diagnostic assumption. These
fresh supplier quotes are observations, not locked prices.

| Supplier | Returned offers | BS-307 V supplier total | Published base + tax | Login + Search elapsed |
| --- | --- | --- | --- | --- |
| FirstTrip | 165 | BDT 41,069.80 | BDT 44,170.00 | 21.4 seconds |
| TakeOff | 270 | BDT 39,866.99 | BDT 44,170.00 | 15.7 seconds |
| TripLover | 225 | BDT 41,069.80 | BDT 44,170.00 | 7.9 seconds |

The matched BS-307 departs at 22:30. TakeOff is BDT 1,202.81 cheaper in supplier
total. Its disabled Search connection explains why this inventory cannot enter
the platform comparison. FirstTrip's observed duration also exceeds its existing
20-second connection deadline. Each supplier returned mixed underlying source
success flags; successful authentication and some inventory do not establish that
every upstream source succeeded.

## Local correction

Search still compares exact original supplier totals before platform pricing for
equivalent fares. Equal totals now use one shared priority constant:
FirstTrip → TripLover → TakeOff. Cheaper TakeOff/TripLover offers beat a more
expensive FirstTrip offer. Existing fare-equivalence and source-reference
preservation rules remain in force; distinct fare conditions remain selectable.

Tests cover tie order, a cheaper TakeOff overriding FirstTrip, all seven active
supplier subsets, persisted source selection, subsequent FareRules routing, and
bulk persistence. Current API documentation reflects the new priority.

The frontend staff schedule grouping previously compared published gross, so
equal-gross offers with different supplier costs could choose the wrong card.
It now compares exact supplier cost within a schedule and applies the same tie
priority when authorized supplier names are present. Customer payable sorting,
gross display/filtering and every alternative's original references are retained.
Rejected-offer regrouping uses the same rule.

Validation: 107 Rust library tests passed (2 opt-in tests ignored), fresh
PostgreSQL integration passed (including all supplier subsets and routing), and
all-target Clippy/formatting passed. Frontend canonical prebooking checks,
TypeScript and changed-file ESLint passed. The separate legacy
`verify-flight-search-reference-safety.mjs` fails its existing line-649 assertion
requiring a direct `triploverCall('Search')`; unchanged HEAD and working files both
have zero such calls. It does not reach its legacy grouping tests.

## Authorized timeout maintenance

After the user explicitly requested 60 seconds, production FirstTrip and TakeOff
deadlines changed from 20 to 60 seconds in one guarded, audited transaction.
Both connection versions advanced from 1 to 2. TripLover stays at 60 seconds,
version 3. All other controls and availability epochs were verified unchanged.
The prior rows are backed up under
`/root/shapontravels-config-backups/supplier-timeout-20260921T090013Z`.
Internal readiness before/after and public readiness passed; API and notification
services remain active. No service restart, deployment or migration was needed.

Production supplier activation and code release are pending explicit authorization. Before
activation, verify the complete platform Search with all three inventories in
the development environment, including existing pricing-coverage checks, and
follow the deployment runbook. FirstTrip and TakeOff remain search-disabled.
No booking, ticket or notification was sent by this work.

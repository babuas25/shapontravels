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

The user subsequently authorized the complete production release. Backend
`06d71a0` and frontend `8fd2fd5` are deployed in development and production;
matching Actions checks/build/deploy jobs and both Vercel deployments passed.
Authenticated canonical supplier-control reads and actual dashboard/Supplier
Control browser checks passed. Production final reads show all three suppliers
configured and Search-enabled at version 3 with 60-second deadlines. The release
itself preserved connection switches; the subsequent enabled state was observed
through the API and UI. No live switch mutation was performed as a smoke test.
See the deployment runbook for release and verification details. No booking,
ticket or notification was created as a test by this work.

## Final production browser verification

After release, a fresh one-adult DAC–SIN Search for 2026-09-29 displayed BS-307 V,
22:30, **via TakeOff**, supplier fare **BDT 39,866.99**. The UI reported 151 total
schedules and nine US-Bangla schedules. No booking/hold/selection was submitted.
This verifies the originally reported cheaper supplier is now represented by the
primary production flight card; the observed fare is not a locked quotation.

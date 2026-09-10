# Same-airline selection through local price acceptance — 2026-09-10

## Implementation

Customer selection remains explicit: if one offer fails RePrice, select another offer with the same plating carrier from Search, then use its own references for FareRules/RePrice and explicitly accept its successful price. No automatic supplier switch, booking or issue is introduced. The existing endpoints support selection; no new alternative-list endpoint or frontend was added.

Known fare-unavailable messages now return 409 FARE_UNAVAILABLE. The captured invalid/expired-session message returns 410 SUPPLIER_SESSION_EXPIRED. Matching is limited to exact evidenced messages after trimming; unknown errors remain 502. Private supplier diagnostics are not exposed to API clients.

Migration 0011 adds flight_offers.reprice_required. A classified revalidation failure sets this flag, retaining old revisions for audit but preventing their reacceptance or use for a new booking. A successful revalidation creates a new revision and clears the flag. A different offer is independent. Existing idempotent booking-result replay is unchanged. FareRules now uses the same complete selected-direction resolver as RePrice.

## Local verification

Disposable PostgreSQL integration tests create two same-airline 6E offers with different Q/V classes and multiple direction options. The sequence verifies:

- Price and accept A, then simulate a supplier fare rejection for A.
- Return 409 FARE_UNAVAILABLE without automatically contacting an alternative.
- Block old A acceptance; reject A/B mixed segment references.
- Explicitly select B, forward only the selected FareRules refs, price B with null returned segment refs, and accept B idempotently.
- Acceptance makes no supplier call, including for bookable=false offers. The prebooking mock panics if Book is invoked.
- B success does not unblock A. Fresh A success creates a new revision; the old A price stays superseded.
- Captured supplier session expiry returns 410 and blocks old acceptance.
- Zero bookings for both offers. Existing Book tests separately verify the rejected-quote guard and direct-issue rejection using mocks only.

Ordinary unit/fixture/HTTP suite, disposable PostgreSQL integration suite, formatting, all-target Clippy with warnings denied and diff whitespace checks passed.

## Live verification

Fresh Triplover production Search via the current local public router and an isolated database. Multicity: DAC–SIN 15 October 2026, KUL–DAC 20 October, DAC–MLE 25 October. Two adults, children aged 3 and 11, one infant; BDT 500 fixed All/All markup per passenger.

The diagnostic selected two separate 6E offers with different first-segment classes. This models customer selection; it is not a production automatic-selection policy. Both happened to be available, so the live run does not claim to reproduce a fare rejection; rejection-to-alternative behavior is verified deterministically by the local tests.

| Stage | Result |
|---|---|
| Search | HTTP 200, 528 offers |
| First 6E fare, class N: FareRules | HTTP 502; supplier `Fare display key not found for Indigo` |
| First 6E fare, class N: RePrice | HTTP 200; group selling total BDT 409,010.66 |
| Alternate 6E fare, class M: FareRules | HTTP 502; same supplier message |
| Alternate 6E fare, class M: RePrice | HTTP 200; group selling total BDT 448,767.56 |
| Accept alternate M quote locally | HTTP 200 |
| Persisted revisions / accepted / bookings | 2 / 1 / 0 |

The two live fares reported bookable=true in Search and RePrice; bookable=false safety was tested locally with mocks, not claimed as live direct-issue verification. Independent Decimal audit passed for all 528 saved selling Search offers and both successful RePrices. Capture completed in 48.42 seconds.

The FareRules supplier limitation remains unresolved. Pricing and local acceptance succeeded, but this is not full fare-rule availability or booking readiness verification. Do not present unavailable fare-rule details as known.

Private raw requests/responses, selected offer snapshots, acceptance output and audit are in `.local/evidence/prebooking-6e-20260910/`. The live transport permits only Search, FareRules and RePrice. Acceptance is local; supplier Book/Cancel/NewTicket/PNR were not called. Booking/ticketing environment flags were false in the test process. No live booking, direct issue, main database migration, deployment or runtime restart occurred.

Apply migration 0011 before deploying this build. Broader pending requirements are unchanged; see [the integration flow](../PREBOOKING_FLOW.md).

# Flight search latency investigation — 2026-09-25

The requested behavior is to preserve results from every enabled supplier.
Reducing supplier deadlines or omitting slow suppliers is not a performance fix
for this requirement.

## Measured observations

- The localhost browser request includes session setup, Rust Search, portal
  pricing reads and presentation mapping. The supplied Server-Timing screenshot
  showed approximately 60 seconds for the backend and 10.72 seconds for pricing.
- The local test database recorded the matching round trips at 16:11 and 16:17
  Asia/Dhaka: Search took 62,028 ms and 61,851 ms. FirstTrip and TakeOff succeeded;
  TripLover failed. Each supplier had a 60-second deadline. These stored outcomes
  distinguish success/failure, not the transport failure type.
- A direct DAC–BKK / BKK–DAC search for 2026-10-10 / 2026-10-30, one adult in
  economy, received TripLover HTTP 200 headers in approximately 11 seconds.
  Local probes then downloaded the approximately 6.7 MB response slowly. One
  bounded probe received only 2,228,224 bytes after 50 seconds. Requesting gzip
  still returned an uncompressed response in that probe.
- The same itinerary, supplier account and endpoints were tested from the
  existing production host without changing its configuration or services.
  Headers arrived in 11,425 ms; the complete 6,631,966-byte JSON arrived in
  25,715 ms and contained 920 offers with supplier success. This measures one
  supplier call, not the complete Rust API or website request. It is a single
  comparison, not a latency percentile or guarantee.

The comparison is consistent with environment/network-dependent transfer latency;
repeated matched tests would be needed to isolate that effect from changing
supplier load. An early HTTP 200 does not mean the complete supplier inventory
has arrived. Supplier/network transfer and portal post-search overhead are
separate contributors.

## Portal pricing consolidation

Canonical portal searches use `GET /api/pricing/search/{id}` to retrieve complete
stored pricing and permitted supplier names together. Previously, 414 offers
required five pricing requests and, for Super Admin, five name requests. Those
requests repeated fresh identity-provider checks.

The new request authenticates freshly, checks the search owner, and returns all
stored offers without the old 100-offer batching boundary. Canonical authority
is rechecked under the identity transaction barrier. Only a current canonical
Super Admin staff session receives supplier names. The frontend verifies the
search ID, complete offer-ID set, currency, pricing audience and permitted name
set before displaying results. Missing pricing and expired searches/offers fail rather
than returning an incomplete map. Legacy portal batching remains available on
its existing path; `/api/pricing/offers` keeps its existing contract.

This reduces website post-search requests. It does not remove supplier transfer
time from direct `/api/Search` clients.

## Local comparison after implementation

One new round-trip Search completed in 31,497 ms and returned 2,642 retained
offers. All three suppliers succeeded in this run. Transport logs measured
TakeOff at 8,275 ms, TripLover at 14,614 ms and FirstTrip at 27,768 ms, confirming
that the slowest supplier and transfer times vary across requests. This binary
was running before the gzip change below; do not attribute this Search duration
to compression or the pricing endpoint.

The same saved search and current canonical Super Admin session were then read
through the old and new pricing paths. The old path used three concurrent batches,
with pricing and supplier-name reads concurrent inside each batch.

| Run | Offers | Old requests | Old time | New requests | New time |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 2,642 | 54 | 5,076 ms | 1 | 573 ms |
| 2 | 2,642 | 54 | 4,248 ms | 1 | 617 ms |

Every pricing value and supplier name matched exactly. The new response was
946,181 bytes. Mean time for these two pricing-read samples fell from 4,662 ms
to 595 ms (approximately 87%). This measurement covers Rust HTTP pricing/name
reads and their provider verification; it excludes frontend session-provider
work and Search itself, and is not a production percentile or end-to-end SLA.

## Compression support and verification

The supplier HTTP client now negotiates and decodes gzip when a supplier supports
it. The streaming size cap is enforced on decoded bytes, so a compressed body
cannot evade the 8 MB / 64 MB response limits. This does not establish that
TripLover currently returns gzip. The Rust server already compresses eligible
outgoing client responses when the client requests gzip.

Verification includes a real-router disposable-database test for 414 complete
offers and one provider lookup, exact equivalence to the existing five pricing
batches, ownership and expiry failures, role visibility, and role revocation
after authentication. All 12 supplier transport tests pass, including gzip
negotiation, exact decoded JSON and decoded-size boundary rejection. Frontend
tests cover 401 offers in one request, account roles, mismatched search/currency,
missing or extra pricing, missing names, and empty searches. Type checking,
linting, OpenAPI reference/privacy checks and operation-ID uniqueness pass.

The gzip debug binary was started on the existing local test backend port
18083 with its existing canonical test profile. Readiness passed. A fresh
authenticated read returned all 2,642 pricing entries and supplier names.
Outgoing gzip was verified: 946,181 decoded bytes transferred as 188,221 bytes.
The temporary benchmark backend on port 18084 was then stopped.

## Complete supplier response investigation

The next investigation separates supplier header wait, body download (including
automatic decompression), and JSON parsing. Successful transport logs now include
`http_version`, `download_ms`, `parse_ms`, and `decoded_bytes`. Existing size
limits, deadlines, retry rules and supplier selection are unchanged. These fields
contain timings and sizes, not credentials or supplier inventory.

Code inspection confirms that supplier calls already run concurrently, and the
shared clients reuse connections and cached authentication. Header timing starts
after authentication and request serialization. It includes connection setup,
request transmission, upstream wait and runtime scheduling; it does not isolate
supplier computation. Offer normalization and database persistence happen after
all supplier tasks complete.

An HTTP/2 candidate was built using reqwest's optional `http2` feature, with
normal HTTP/1.1 fallback. The same local profile, account, dates and itinerary
were tested sequentially in HTTP/1.1, HTTP/2, HTTP/2, HTTP/1.1 order. The original
backend stayed on port 18083 and the candidate used port 18084. No searches were
run concurrently with one another by the benchmark.

| Sample | Supplier protocol | Full API HTTP time | Slowest supplier complete | FirstTrip headers | FirstTrip body + parse | Returned offers |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | HTTP/1.1 | 26,794 ms | 23,130 ms | 15,857 ms | 7,271 ms | 2,718 |
| 2 | HTTP/2 | 21,218 ms | 17,358 ms | 15,267 ms | 1,891 ms | 2,748 |
| 3 | HTTP/2 | 20,777 ms | 17,130 ms | 15,852 ms | 1,276 ms | 2,739 |
| 4 | HTTP/1.1 | 20,453 ms | 16,019 ms | 15,117 ms | 901 ms | 2,623 |

All three suppliers succeeded in every sample. Every candidate supplier response
negotiated HTTP/2. FirstTrip was the slowest supplier in all four samples. Existing
scope rules excluded 24 offers per sample, so `x-search-partial` remained true;
the flag did not represent a supplier failure. Existing selection also affected
the final count. Inventory varies across live requests, so these are not identical
payload comparisons or a latency percentile.

The candidate measured JSON parsing at 177–221 ms per supplier for 6.8–8.3 MB
decoded bodies. Download times were 457–3,806 ms. A separate direct TripLover
ABBA probe measured mean full-response times of 11,510 ms over HTTP/1.1 and
11,546 ms over HTTP/2, with no gzip response. The attempted direct authentication
probes for FirstTrip and TakeOff returned HTTP 403; no bypass was attempted.
Their successful application transport was used for the comparison above.

There is no demonstrated consistent protocol speedup: the second HTTP/1.1 run
was fastest, and its first run had an unusually long FirstTrip body transfer.
FirstTrip's 15.1–15.9-second header wait was essentially unchanged across modes.
The HTTP/2 candidate and its additional dependencies were therefore reverted;
the gzip support and detailed timing remain. The candidate passed 117 library
tests (two existing ignored) and the Clerk HTTP adapter loopback contract, but
enabling HTTP/2 would also affect unrelated reqwest clients.

Further material improvement needs evidence from the supplier/server network
boundary, particularly FirstTrip's header delay and variable body throughput.
Client-side parsing changes cannot recover the measured 15-second header wait.

After reverting the protocol candidate, the final binary built successfully,
all 12 supplier transport tests passed again, and formatting/diff checks passed.
It replaced the local test backend on port 18083 with the existing profile;
readiness returned `ready`. The temporary candidate on port 18084 was stopped.
The active local log is `/tmp/shapon-supplier-timing-active.log`. No supplier
latency improvement is claimed from the diagnostic logging change.

## Follow-up: observed 34.26-second DAC–SIN website search

The user's next screenshot reports DAC–SIN on 2026-10-20 and SIN–DAC on
2026-10-30, one adult in economy. Browser total is 34.26 seconds, including
session 1.57 seconds, backend 31.97 seconds, pricing 664 ms and mapping 15 ms;
browser content download is 10.48 ms. This is a different itinerary from the
earlier DAC–BKK protocol benchmark.

The local database confirms the same route/dates in usage
`88304aa2-130b-4247-93f0-62837c94e636`, started at 2026-09-25 17:51:46 Asia/Dhaka.
All three supplier outcomes are successful. Its transport logs show:

| Supplier | Authentication | Headers | Download | JSON parse | Complete supplier task |
| --- | ---: | ---: | ---: | ---: | ---: |
| TakeOff | 550 ms | 8,456 ms | 483 ms | 162 ms | 9,655 ms |
| TripLover | 210 ms | 17,995 ms | 1,121 ms | 247 ms | 19,576 ms |
| FirstTrip | 709 ms | 26,456 ms | 1,341 ms | 250 ms | 28,759 ms |

Calls overlap, so these supplier totals must not be added together. The request
waits for FirstTrip to finish. Preparation is 134 ms and persistence is 1,659 ms;
the processing log totals 30,564 ms. The usage record including later response
encoding/history work is 31,097 ms. The frontend's 31.97-second backend phase
also includes authentication, HTTP transfer and response handling outside those
logged processing stages. Do not assign that remaining difference to any one
operation without more instrumentation.

In this observed request, FirstTrip's header wait dominates; receiving/parsing
its body takes a further 1.592 seconds. The previous pricing reduction is visible
in the screenshot, but it does not remove the supplier wait. Existing scope
validation excluded 180 offers; the partial flag does not indicate a failed
supplier in this run. No new latency claim or protocol/configuration change is
made from this observation.

## Optimized local runtime comparison

The localhost backend was running the unoptimized Rust debug build. The existing
deployment workflow already builds production/development server artifacts with
`cargo build --release`; changing the local runtime does not establish a new
production performance gain.

To isolate application overhead from supplier variability, 2,482 stored original
offers from the observed DAC–SIN search were replayed through the real router
and PostgreSQL using the offline `examples/search_load.rs` adapter. These are the
previously retained snapshots, not the full original 2,672-offer supplier set.
Minimal supplier metadata was reconstructed for the replay. The example now
accepts optional `LOAD_REQUEST_FILE` so the request matches those saved offers;
its existing default request and local/empty-database safeguards remain.

Both binaries used the same source, fixtures, request and gzip setting, with one
request per freshly created disposable database. Three samples per profile were
run in debug/release/release/debug/debug/release order. Preliminary measurements
potentially overlapping an independent microbenchmark were excluded; the table
contains the subsequent confirmation without that overlap.

| Build | Full replay HTTP times | Median |
| --- | --- | ---: |
| Debug | 3,288 / 3,325 / 3,393 ms | 3,325 ms |
| Release | 682 / 671 / 681 ms | 681 ms |

The measured median saving is 2,644 ms (about 79.5%) for this local application
workload. It includes fixture JSON parsing, offer processing, PostgreSQL writes,
response encoding and gzip, and excludes supplier network wait, external identity
verification, frontend session/pricing and browser rendering. It is not a 79.5%
reduction of the user's complete flight search. All six runs returned and
persisted 2,482 offers, with identical normalized business fingerprints and
16,737,498 decoded response bytes. Generated references differ by design; gzip
wire lengths therefore vary slightly. All temporary databases were removed.

Median projection time fell from 942 to 94 ms, SQL JSON encoding from 577 to
16 ms, and persistence overall from 2,005 to 562 ms. SQL execution itself was
similar at 450 versus 442 ms. Small alternative code changes were measured and
not retained: duplicate frontend pricing validation saved about 7 ms for 2,500
offers, and a reference traversal prototype saved about 31 ms in debug.

The active local backend on port 18083 now uses
`target/release/shapontravels-api`. Its existing canonical profile and local
launcher were updated to select that binary on restart; the other local profile
was left intact. Readiness returned `ready`; formatting and diff checks passed.
The active log is `/tmp/shapon-supplier-release-active.log`.

One authenticated live verification of the same DAC–SIN dates completed in
28,489 ms after the switch. All three suppliers succeeded: TakeOff 4,898 ms,
FirstTrip 25,791 ms and TripLover 27,445 ms. Preparation plus persistence took
511 ms after supplier completion. It returned 2,403 offers; 180 were excluded
by the existing scope rules. Inventory and supplier timings differ from the
earlier live search, so its total is not an isolated before/after speedup or a
guarantee for the next request. This is direct API time, excluding frontend
session/pricing. Supplier wait still dominates.

Private fixtures, six logs and result summaries are Git-ignored under
`.local/evidence/search-local-release-20260925/`. To repeat, build the example in
debug and release, set `LOAD_CAPTURE_DIR` to that directory and
`LOAD_REQUEST_FILE` to its `request.json`, request gzip, and use a newly created
empty local database ending `_load_test` for each run. No supplier requests are
made by that replay adapter. No production deployment was performed.

## Search click startup (frontend)

The search panel previously only navigated to `/flights`; the browser search POST
started in `FlightResults` after the server page and client component loaded.
The canonical results page also repeated an identity lookup through `apiActor`
after resolving the authorized dashboard session. Submitting identical criteria
did not change the results component key and could leave the previous results.

The frontend now starts the authenticated POST in the click handler before
navigation, displays a disabled `Searching…` button during navigation, and hands
the original promise to the results page. A new UUID per click refreshes even an
identical search. The handoff is browser memory only, consumed once, and requires
the exact search input and the same Clerk user and session. It holds at most four
entries for 270 seconds; direct navigation or an unavailable handoff uses the
normal request path. Search API authorization and complete supplier collection
remain in place. Results date navigation also starts its request immediately.
The canonical page reuses its freshly authorized session for presentation;
legacy actor resolution remains unchanged. A route loading fallback supplies
feedback while the server page is loading.

Verification on local `development`:

- `node scripts/verify-flight-search-start.mjs`: immediate POST, one-time handoff,
  input/session isolation, unique attempts, bounds/expiry, original errors and
  server/anonymous behavior passed.
- `node scripts/verify-flight-search-page.mjs`: canonical role gates, legacy
  authority, attempt validation and same-criteria component keys passed.
- `node scripts/verify-rust-prebooking.mjs`, TypeScript no-emit checking, targeted
  ESLint and `git diff --check` passed.
- In Chrome, modifying the existing DAC–SIN one-way 2026-09-30 search and
  submitting unchanged criteria immediately showed a disabled `Searching
  flights` button, then a new attempt URL and 126 grouped flights. The corresponding
  backend log contained one completed Search (`0673ee70-163c-438b-adc0-8d6019ab52ef`),
  633 returned offers and three successful suppliers with none failed/blocked.

This removes page-navigation work from the path before starting the browser POST.
No exact click-to-network millisecond measurement or guaranteed supplier latency
reduction is claimed. The changes are local and have not been deployed.

## Supplier follow-up prepared for the operator

Draft subject: **Search API complete-response latency and compression**

For DAC–BKK on 2026-10-10 and BKK–DAC on 2026-10-30, one adult in economy, we
need all available supplier results in the final response. In application tests
on 2026-09-25 around 11:28–11:30 UTC, FirstTrip headers arrived in 15.1–15.9
seconds and its body plus JSON parsing took a further 0.9–7.3 seconds. Payloads
across the three suppliers were approximately 6.8–8.3 MB. HTTP/2 testing did not
demonstrate a consistent improvement.

Please correlate the time from request arrival to complete response with your
upstream search timings, and confirm whether Search JSON gzip compression can be
enabled. Please identify any supported endpoint or response mode that reduces
latency/payload size while preserving the full inventory and booking/repricing
references. We can repeat a coordinated measurement with supplier request IDs
and response-byte counts once you identify the relevant tracing header.

This draft contains no credentials, bearer tokens, customer identities or full
supplier payloads. No message has been sent to any supplier.

## Release scope

Work is on local `development` branches. No production release, identity-pin
change, database migration or supplier timeout change is part of this work.
Publish the compatible backend before the dependent frontend using the deployment
runbook, when a release is authorized.

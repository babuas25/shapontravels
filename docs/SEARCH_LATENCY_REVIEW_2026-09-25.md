# Flight Search latency review — 2026-09-25

Production runtime: `54e5c304e968a30f1e0ecba085f299be21003701`.
The API was active and `/health/ready` returned 200 during this review.
These measurements come from the production API's structured service logs on
2026-09-25 UTC. They contain no supplier credentials or passenger details.

## Findings

All active suppliers are dispatched concurrently. Search dispatch took 8–22 ms
in the measured examples. The frontend starts the authenticated request in the
search click handler and hands that same promise to the results page; the page
does not start a second request for a matching attempt.

The supplier transport already reuses a `reqwest::Client`, caches login tokens,
requests gzip, and bounds TCP connection setup to five seconds. On 111 successful
supplier Search responses (all HTTP 200), median authentication time was zero.
The time spent waiting for response headers dominated body download and parsing:

| Supplier | Calls | Header wait median / p90 | Body time median / p90 |
| --- | ---: | ---: | ---: |
| Firsttrip | 33 | 4.6 / 27.9 s | 0.24 / 0.49 s |
| Triplover | 38 | 5.0 / 17.7 s | 0.30 / 0.73 s |
| Takeoff | 40 | 5.0 / 7.8 s | 0.37 / 0.55 s |

The header wait includes connection setup and supplier processing; it does not
separate those phases. As a limited network control, unauthenticated HEAD calls
to each Search endpoint from the production VPS completed DNS and TLS setup in
about 0.3–0.6 seconds. Those HEAD calls do not benchmark an authenticated
Search POST, but they do not indicate a 30-second connection setup problem.
The VPS had low load and ample available memory at the time of review.

Two later single-supplier Takeoff searches completed in 4.0 and 7.2 seconds
inside Rust. Their supplier waits were 3.6 and 6.8 seconds; offer persistence
was about 0.35 seconds each. This shows why the browser can still take longer
than a supplier's 4–5-second response: the portal creates a short-lived search
session before Search, then retrieves complete pricing after Search.

Two completed searches with all three supplier results show the round/multi-city
critical path. Both were single adult, economy searches:

| Search | Approximate UTC window | Takeoff | Triplover | Firsttrip | Offer persistence | Rust total |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| DAC–SIN 20 Oct, SIN–DAC 30 Oct | 14:22:31–14:23:05 | 8.8 s | 20.1 s | 30.1 s | 3.6 s | 33.9 s |
| DAC–SIN 20 Oct, SIN–KUL 24 Oct, KUL–DAC 30 Oct | 14:23:54–14:24:30 | 8.5 s | 9.0 s | 33.0 s | 2.7 s | 35.8 s |

Search offer persistence is the largest local cost after the last supplier
returns. The current 64-offer batch writes took 40 and 26 SQL statements in
these examples; SQL execution accounted for 2.9 and 2.2 seconds respectively.
Further batch or storage changes need a representative database benchmark and
must preserve atomic writes and offer snapshots. The development VPS did not
respond over SSH during this review, and the local PostgreSQL test listeners
were unavailable, so no database write benchmark was run.

## Next actions

1. Use the new per-search `usage_id` and attempt number on supplier authentication,
   header, and body timing events to identify the exact slow phase for each
   completed round-trip and multi-city search after a reviewed deployment.
2. Ask Firsttrip and Triplover to correlate the UTC windows and route/date
   combinations above with their Search processing and downstream GDS timings.
   Request a faster complete-result path or a supplier-side performance fix;
   do not remove suppliers or fares to make the response appear faster.
3. Benchmark larger atomic offer batches against representative 1,500–2,700
   offer payloads in an isolated PostgreSQL database before changing batch size.
4. Recheck frontend `Server-Timing` after supplier changes. Session creation and
   complete pricing lookup add time after the browser click, but the 30-second
   supplier wait dominates these round-trip and multi-city examples.

No supplier contact, production deployment, database migration, or production
configuration change was made as part of this review.

## Local portal-path improvement (not released)

The canonical portal now receives its verified role with the search session,
removing a separate identity-session read. It requests the complete pricing
snapshot in the Search response, removing a follow-up HTTP request. Rust checks
the provider and current database authority again after supplier work before
including that snapshot. Rust verifies stored ownership; the frontend validates
every offer's pricing, currency and supplier-name visibility as before. Old
backend responses retain the existing identity and pricing fallback. All
suppliers and all returned fares remain in the result. The development VPS is
off and there is no local test PostgreSQL listener, so a production latency gain
has not yet been measured.

Local verification: Rust formatting, all-target type checking and strict Clippy,
117 unit tests, integration-test compilation and optimized release build passed.
The matching frontend passed lint, TypeScript, its prebooking fixture checks and
the optimized Next build. The database-backed inline-pricing assertion could only
be compiled because the isolated PostgreSQL test database is unavailable.

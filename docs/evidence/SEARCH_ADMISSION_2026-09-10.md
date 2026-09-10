# Bounded Search admission — 2026-09-10

## Problem and behavior

Multiple Search requests previously overlapped supplier JSON parsing, pricing, snapshot persistence and full response buffers without a dedicated admission bound. The controlled VPS replay showed that simultaneous work materially raises memory and latency; its isolated PostgreSQL process also required more than a 256 MiB budget at four concurrent requests.

The first draft admitted two expensive Search responses with four queued slots and a two-second wait. After the user clarified 10-12 simultaneous customers, the local default was revised to four active responses and eight waiting requests; the user subsequently capped queue wait at two seconds, rejecting the intermediate 90-second proposal; see the [burst follow-up](SEARCH_BURST_2026-09-10.md). Environment variables configure these limits within explicit bounds. Acquisition happens after authentication/permission/request validation and before current supplier/pricing snapshots. No supplier call or inventory insert happens on overload: the entire request returns 503 SEARCH_BUSY and Retry-After: 1. Existing authentication rate accounting still applies. No offer truncation, new deduplication, shared snapshot cache or pricing/reference change is introduced.

The permit covers JSON serialization and the response body outside the gzip layer. Returning headers does not release capacity while a large response remains buffered for compression/consumption. Completion/error/drop and waiting/work cancellation release it. Slow clients can hold slots; queue limits/deadlines reject excess work rather than accumulate an unbounded inventory backlog. This is not a global byte limit, total request timeout or cluster-wide quota. Each replica needs capacity budgeting, and HTTP/socket buffers are outside the gate's accounting.

## Capture correction

The original [summary audit](SEARCH_SUMMARY_2026-09-10.md) covered **7,207 offers across 12 scenarios**, not one Search with 7,207 offers. The saved summary confirms the largest retained response was the all-supplier roundtrip with **2,177 offers** (2,184 source offers). The prior VPS report's references to a larger 7,207-offer single-request workload have been corrected. No synthetic 7,207-offer workload is represented as a real capture.

## Local replay comparison

Same current release binary, exact prior roundtrip capture, four concurrent arrivals, gzip, three samples per active limit, alternating order 4/2/1 then 1/2/4. Each sample uses an empty disposable local PostgreSQL 18 database. For this all-success scheduling comparison, queue capacity is four and wait budget is explicitly **30 seconds**, unlike the first-draft default two seconds (subsequently revised to 90 seconds). Limits 4, 2 and 1 change only the amount of admitted overlap; four active permits allow all four arrivals immediately.

| Active limit (four arrivals) | Median process peak RSS | Median HTTP full-body time | Median complete-wave time |
| --- | ---: | ---: | ---: |
| 4 | 1,051.55 MiB | 1.921 s | 2.654 s |
| 2 | 596.30 MiB | 2.003 s | 3.421 s |
| 1 | 574.66 MiB | 2.710 s | 4.779 s |

Two active requests reduce median replay RSS about **43.3%**, while full-wave completion takes about **28.9% longer**. One active slot saves only about 3.6% more RSS than two and takes about 39.7% longer per wave in this experiment. This initially supported two as a provisional tradeoff for that four-arrival experiment. The later twelve-arrival requirement supersedes that draft default; neither experiment establishes production throughput.

All 36 requests across nine samples returned and persisted exactly 2,177 offers each, with the same prior stable business fingerprint and 24,446,992 decoded bytes per response. Each sample persisted 8,708 offers. Normalized fingerprints exclude references/timing; full original/selling, reference/ownership and exact-decimal semantics remain covered by integration/fixture tests.

These are local macOS process measurements using `/usr/bin/time -l`, not new VPS measurements or production latency predictions. RSS includes replay fixtures and simulated client decoding/assertions, and excludes the separate PostgreSQL process. No direct reduction in PostgreSQL RSS, database storage, CPU per Search or individual response size is claimed. Arrivals with a short configured queue deadline may receive SEARCH_BUSY when a live supplier call holds capacity longer; the all-success replay does not contradict that intended overload policy.

## Verification

- Unit tests exercise bounded queue/full rejection, timeout and cancellation release, reuse after a slot opens, compression body lifetime, disconnect and erroring-body release, and active-work cancellation.
- Real authenticated PostgreSQL integration holds a successful gzip Search response open, verifies a waiting Search times out as 503 SEARCH_BUSY with Retry-After/no-store/request-ID, and proves supplier call count and persisted-offer count do not increase. Unauthenticated and invalid requests retain their normal errors and health remains available while capacity is occupied. Body completion/disconnect permit the next Search.
- Formatting, all-target clippy with warnings denied, all 42 unit/foundation/fixture tests and PostgreSQL integration pass (integration: 37.02 seconds). Configuration tests reject invalid bounds and accept zero queued slots. Existing exact decimal, projection, reference, transaction rollback and cleanup/scheduler checks also pass.
- No live supplier call or production mutation is used. No migration is needed. All disposable admission/replay databases were removed. The change is local and requires user approval before commit/push/deployment.

Reproduction data and nine replay logs: `.local/evidence/search-admission-20260910/`. The example accepts LOAD_MAX_ACTIVE, LOAD_MAX_QUEUED and LOAD_QUEUE_WAIT_MS; the default router and serve-mode configuration both apply admission controls.

The final two-second maximum rejects queue budgets above 2000 ms. Earlier 30/90-second measurements are historical scheduling experiments, not validation that all burst requests succeed under the final short deadline.

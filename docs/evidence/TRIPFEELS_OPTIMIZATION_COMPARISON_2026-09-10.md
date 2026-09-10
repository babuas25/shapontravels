# Tripfeels comparison: further optimization without losing offers

Reviewed 2026-09-10. Shapon baseline: `3fb452b22e5dcd1584b02d66c3bd73384ca841a4`; Tripfeels source: `93e14180c46d9d347b86bf09a428d151d62fc0eb`. Both working trees were clean before review. Tripfeels was inspected read-only; no service, credentials or supplier API was used. This is a source/architecture comparison, not a benchmark claiming Tripfeels is faster or smaller on the same workload.

## User priority

Further optimization is important and must preserve all valid offers/options under our existing business contract. Do not introduce top-N cutoffs, stronger fare merging, dropped fields or lost precision to make the response appear smaller. Existing approved equivalent-fare lowest-supplier selection remains unchanged. This review records options; it does not change runtime behavior or deploy a new optimization.

## What the Tripfeels source actually does

| Technique | Evidence in Tripfeels | Shapon comparison and suitability |
| --- | --- | --- |
| Negotiated gzip | `src/main.rs:212–217`, `Cargo.toml` enables `tower-http/compression-gzip` | Shapon has no application compression middleware. Strong first candidate: byte-preserving transport compression, keeping every JSON field/offer. CPU and Vary/Accept-Encoding behavior need integration tests. |
| Supplier response decompression | `Cargo.toml` enables reqwest gzip/brotli/deflate; shared client in `src/main.rs:96` | Shapon already reuses HTTP clients, but its reqwest features omit these codecs. Supplier traffic can shrink if upstream supports compression; decoded response-size limits must still be enforced. Not a decoded-RAM reduction by itself. |
| Reuse repeated Search results | `src/aggregator/search.rs:822–899` checks process cache then Redis before fanout; `src/config/mod.rs:85` defaults search cache TTL to 180 seconds | Shapon reprocesses and persists every request. Reuse could reduce repeated work, but this project's freshness policy cannot inherit 180 seconds. Keep principal/pricing context, enabled suppliers/epochs, currency, child ages, request fields and absolute expiry in cache design. |
| Share immutable offer snapshots | `src/aggregator/search.rs:157–187`, `:1033–1068`, `:7673`; `src/reliability.rs:200–235` stores one Redis snapshot plus small search aliases | Shapon stores full original and selling JSON per retained offer per Search. Shared raw snapshots and small per-client mappings could reduce repeated storage. Mutable RePrice/acceptance/Book state must remain private and durable. Rebinding public IDs must not share client authorization. |
| TTL plus scheduled cleanup | `src/aggregator/search.rs:106`, `:780–800`, `:10038`; `src/reliability.rs:127–146` clamps absolute deadlines; max offer TTL is 15 minutes | Shapon has 10-minute reference expiry and indexes but no periodic cleanup. Expired unbooked/unreferenced search data cleanup controls accumulation; preserve any data referenced by quotes, bookings, reconciliation and required audits. Do not copy Tripfeels' 15-minute lifetime. |
| Search inventory is transient; pricing/booking state is durable | `src/aggregator/search.rs:7585–7600` explicitly bypasses PostgreSQL for AirShopping snapshots; storage is process memory/Redis, selected offers later persist | This reduces synchronous writes there, but is an architectural change here: our follow-up endpoints depend on PostgreSQL search/offer rows. Redis is another RAM consumer on the 4-GB VPS, not free memory. Prefer staged snapshot deduplication/cleanup before replacing durable search storage. |
| Background cache/audit writes | `src/aggregator/search.rs:1081–1090`, `:8360–8420` | Can shorten response latency for optional cache/audit copies, but detached work still consumes resources and may be lost on crash. Shapon must not return usable references before their authoritative ownership/snapshot data is durable. Bound queued background work. |
| Display-only canonical response | `src/aggregator/search.rs:9826–9888` builds `offersGroup`; `src/models/flight.rs:398–408` keeps selection/raw supplier state outside the display model | Tripfeels' smaller display contract is not interchangeable with our `item1.airSearchResponses` shape. Copying this DTO would discard/restructure fields even if fare options remain accessible. Not suitable under current Shapon requirements. |
| All fare groups rather than arbitrary top-N | `src/aggregator/search.rs:985–1007`, `:9826–9848` constructs every grouped display entry | Grouping is a distinct business representation, not permission to collapse our distinct options. Triplover mapper `src/providers/triplover/search.rs:635–659` also expands direction combinations, which can increase cold-search work. Its `.take(10)` at :183/:193 limits diagnostic logging, not returned offers. |

The inspected cache-miss Search path proceeds to supplier fanout without a per-search-key in-flight lock. Cache hits should not be confused with singleflight: concurrent identical misses may still duplicate work. Adding bounded singleflight is a separate design candidate, not a feature credited to Tripfeels here.

## Direct lossless-compression proof on our response

Used the exact `body` JSON substring from `.local/evidence/search-summary-20260910/roundtrip-all-public-search.json`, without parsing/reserializing that substring. Input: 24,446,992 bytes and 2,177 offers. Python gzip was run three times per level on this Mac; decompression was asserted byte-for-byte equal to the input.

| Gzip level | Input | Compressed bytes | Size reduction | Median local encode time |
| --- | ---: | ---: | ---: | ---: |
| 1 | 24,446,992 | 1,911,723 (~1.82 MiB) | 92.18% | 39.9 ms |
| 6 | 24,446,992 | 1,148,213 (~1.10 MiB) | 95.30% | 81.8 ms |

Every byte, numeric lexeme, field and offer is recoverable. This demonstrates compressibility, not deployed HTTP performance or the exact speed/ratio of Tower/Nginx streaming compression. The timings are local compression-only timings, not 2-CPU VPS latency. Gzip lowers transferred bytes, not the fully decoded JSON tree's RAM. Private result JSON: `.local/evidence/tripfeels-comparison-20260910/gzip-proof.json`.

## Remaining Shapon-specific avoidable work

1. `src/search.rs:398` calls full `projection::single_component(..., Fixed(0))` for every source offer merely to validate it. `src/projection.rs:49` clones the offer. Separate read-only coverage validation from output projection, preserving all validation checks. Only selected offers need a selling projection.
2. `src/selection.rs:33` clones an entire candidate to strip fields, then serializes and retains full equivalence keys (`:134`). A borrowed canonical-key serializer, or collision-safe digest indexing with exact equivalence confirmation, can avoid cloned trees/large resident key strings. Preserve unknown fields, missing/null distinctions, booking-class rules and deterministic priority. A hash alone must never authorize merging different fares.
3. Larger redesign: retain immutable raw offer storage plus sparse pricing/reference overlays and serialize the supplier-compatible public shape at the boundary. That could avoid simultaneous complete original/selling trees and repeated PostgreSQL snapshots while retaining all information. It requires explicit equivalence tests for response shape, exact decimals, follow-up references and historical accepted prices; savings are not measured yet.
4. Bound concurrent expensive work or use backpressure to control peak process memory. This schedules whole requests; it must not truncate offers. It does not make one request cheaper and needs load/timeout/error-contract validation.

No further RAM percentage is promised without an A/B benchmark. Tripfeels also retains canonical graphs and display templates, clones some selected/fallback data and materializes direction combinations. It is not universally lighter simply because it uses Redis or Rust.

## Recommended sequence

1. Negotiated response gzip with lossless equality, all-offer counts, unauthorized/error response handling, Vary and accepted/unaccepted encoding tests; benchmark gzip speed vs VPS CPU. This has the strongest measured immediate bandwidth benefit.
2. Remove validation-only projection clones and cloned/string equivalence keys. Replay identical cold/4-concurrent workloads and verify complete offer/reference/pricing semantics before deployment.
3. Agree and implement expired-search cleanup with booking/quote/audit dependencies preserved. Measure table/TOAST/index growth under sustained tests.
4. Prototype shared immutable snapshots and bounded identical-request reuse. First determine supplier session/reference reuse and freshness semantics. Preserve ownership, supplier disable epochs, pricing versions, original absolute expiry and per-client acceptance state. Bound cache bytes and include Redis in total VPS memory measurements.
5. Only after those results decide whether a Redis-first Search architecture is justified. Do not switch to Tripfeels' display schema or discard offers to achieve smaller output.

No source modifications were made to Tripfeels. Only requirements/review notes and a local compression proof were produced for this request.

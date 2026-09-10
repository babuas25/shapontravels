# Search response memory — local verification, 2026-09-10

Release update: subsequently authorized and deployed as `b08b34d` on 2026-09-10; see [verified release](SEARCH_MEMORY_DEPLOY_2026-09-10.md). Local/deferred status below records the original milestone.

The user authorized continuing memory optimization and explicitly deferred push/deployment. This work builds on the uncommitted 16-row persistence batches; neither change has been deployed.

## Investigation and change

The earlier 7% replay-process RSS increase with smaller SQL batches was not a standalone API-memory measurement: the same process runs mock suppliers, the API, gzip collection and client JSON parsing/fingerprint validation. Two diagnostic four-request profiles did not reproduce that increase consistently. Largest sampled client-parsed RSS was 1,106.64 MiB with 64-row batches and 1,029.41 MiB with 16; these are single instrumented runs, not a replacement benchmark. The earlier increase remains a valid observation for its samples, not proof of a production API regression.

The response encoding path nevertheless retained the complete selling offer tree while allocating the complete output JSON buffer. Added a private, single-use `Serialize` wrapper for Search responses. It consumes the outer envelope and `item1` in their existing map order, serializes each `airSearchResponses` offer with serde's normal implementation, then immediately releases that offer tree. Every field and offer is encoded. Other values, including unknown shapes and nested fields named `airSearchResponses`, retain standard serde behavior.

Axum's existing `Json` response still buffers all JSON before returning the response. There is no partial response streaming, changed error response construction, compression change or release of the admission permit during serialization. The database transaction still commits before response construction. No caching, reference sharing, expiry/pricing policy, schema or supplier execution change is introduced. Debug-only phase markers bracket response encoding; production INFO logs do not collect RSS.

The wrapper intentionally cannot be serialized twice: the one-use response construction site owns it, and a repeat attempt returns a serialization error instead of an empty success. It is not exposed as a general-purpose reusable DTO.

## Paired replay

Baseline is the previous local 16-row-batch release binary. The candidate adds consuming response serialization and disabled-by-default encoding phase markers. Both use the same captured 2,184-source/2,177-retained roundtrip workload, gzip and final four-active/eight-queued/two-second admission settings.

Three alternating samples per variant at one/four concurrent requests: twelve runs, 30 successful responses, 65,310 persisted offers. Each run starts the same separate local PostgreSQL 18.3 cluster with a fresh empty database, 32 MiB shared buffers, 1 MiB work memory and 16 connections. No builds/tests overlapped measurement. No supplier network requests or production writes were made.

| Median metric | One Search: before → after | Four concurrent: before → after |
| --- | ---: | ---: |
| Replay process peak RSS | 353.50 → 335.92 MiB | 1,250.28 → 1,133.53 MiB |
| HTTP full-body time | 891 → 894 ms | 1,326 → 1,390.5 ms |
| Complete-wave time including client validation | 1,151 → 1,154 ms | 2,074 → 2,052 ms |
| Sampled sum of PostgreSQL RSS | 124.00 → 123.78 MiB | 341.16 → 340.77 MiB |

Replay peak RSS fell approximately 5.0%/9.3%. Four-request HTTP median increased about 4.9%, while whole-wave time fell about 1.1%; no speed improvement is claimed. Timing variation includes SQL execution (835.5→878.5 ms median at four concurrent requests), despite unchanged SQL implementation between these variants. The samples do not isolate a causal serialization-latency penalty. Four-request peaks ranged 1,157.25–1,413.42 MiB before and 1,106.58–1,169.03 MiB after; scheduling/allocator noise and overlapping ranges limit the conclusion.

`/usr/bin/time -l` measures the entire replay process, including mock fixtures and simulated clients, not an isolated deployed API. PostgreSQL RSS is separately sampled with `ps`; sums double-count shared pages and are not cgroup/unique physical memory. These are small local Mac samples, not VPS capacity verification or a guarantee for twelve simultaneous requests. The cold supplier-parsed trees still dominate much of memory use. No response-size, database-growth or repeated-search-work reduction is claimed.

All responses retained exactly 2,177 offers, 24,446,992 decoded bytes and the prior stable business fingerprint. The fingerprint excludes dynamic references/timing; full original/selling/reference correctness is additionally covered by integration tests. Gzip remains negotiated by existing middleware. Database relation-size medians were unchanged between variants.

## Verification and evidence

- Exact-byte comparisons against standard `serde_json::to_vec` cover all three sanitized supplier Search fixtures, unknown nested fields, Unicode/escaping, null/missing/empty/unrecognized shapes and the precise numeric lexeme `9007199254740993.00500`.
- Formatting, strict all-target Clippy, release replay build and all 45 ordinary unit/foundation/fixture tests pass.
- Full disposable PostgreSQL integration passed in 36.97 seconds, covering Search snapshots/reference ownership, gzip, admission, rollback, prebooking and cleanup. Its database and the stopped profiling cluster were removed afterward.
- Runner, saved before/after binaries, twelve raw replay logs, RSS samples, numerical summaries and the initial diagnostic phase profiles are retained under ignored `.local/evidence/search-app-memory-20260910/`. Reproduce from the project root with `python3 .local/evidence/search-app-memory-20260910/measure.py` and the existing capture directory. In its output, legacy `batch: before/after` labels identify serialization variants; both use 16-row batches.
- Shared snapshot/cache design remains separate because supplier-reference reuse and freshness cannot be inferred from these replays. Constrained VPS verification remains necessary before claiming improved production concurrency. Push/deployment remain deferred by the user.

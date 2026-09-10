# Search performance replay — 2026-09-10

## Workload and method

The 7,207 earlier offers were a total across 12 scenarios. This benchmark replays one larger all-supplier roundtrip response through the real authenticated Axum router and PostgreSQL persistence: 2,184 source offers, 2,177 returned offers, 25,470,124 input JSON bytes and 24,446,992 response bytes (23.3 MiB). Five passengers and fixed BDT 500 per passenger match the saved production Search capture.

No supplier network or mutation calls: `examples/search_load.rs` implements a local-capture-only supplier. Every run creates an empty disposable database. Before/after release binaries use the same harness and saved inputs. Three runs per version/concurrency level, alternating version order, produce the medians below. The preliminary runs and failed harness setup are excluded. `http_ms` includes local API processing/serialization but excludes client-side verification; supplier network latency and internet transfer are absent. `/usr/bin/time -l` reports process-wide peak RSS and CPU. RSS includes fixture storage and response verification, so these figures are comparative replay-process costs, not pure API RSS or proven VPS capacity. CPU excludes the separate PostgreSQL process.

Host: local macOS, 16 GiB RAM, 10 logical CPUs; harness runtime has four worker threads and PostgreSQL is local. Read-only VPS inspection found 2 CPUs and 3,915 MiB RAM, with 3,422 MiB available at that moment. The production process was idle after deployment, so its small idle RSS is not a loaded-memory measurement. No production concurrent load test was run.

## Median results

| Concurrent searches | Version | HTTP latency (ms) | Process peak RSS (MiB) | Persistence phase (ms) | Process CPU seconds |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | before | 1236 | 1015.8 | 789 | 1.16 |
| 1 | after | 1048 | 524.8 | 774 | 0.95 |
| 4 | before | 1886.5 | 3276.0 | 1120.5 | 5.40 |
| 4 | after | 1540.0 | 1819.1 | 1079.5 | 4.40 |

Peak replay RSS fell about 48% for one Search and 44% for four concurrent Searches; median HTTP latency fell about 15% and 18%, respectively. The persistence phase includes projection, summary work, JSON binding and transaction execution, not just server-side SQL. Its improvement on a local database was modest (about 2–4%); no large SQL-time speedup is claimed. Network-separated database roundtrip savings were not benchmarked.

## Changes

- Move supplier inventories out of their envelopes instead of cloning full offer arrays. Subsequent envelope/summary clones contain metadata only.
- Move final response/status arrays into JSON rather than serialize-copy them again.
- Insert retained offers in batches of at most 64 rows within the existing transaction. The 2,177-offer workload requires 35 offer INSERT statements instead of 2,177; each full batch has 640 bound parameters. JSON arguments are bounded by row count, not a byte-size guarantee.
- Record non-sensitive counts and phase durations in the `search_performance` log target. No payloads, credentials or source references are logged.

## Correctness and remaining costs

Every measured request returned HTTP 200, 2,177 offers and the same response byte count and normalized non-reference fingerprint. Unit/fixture tests and PostgreSQL integration additionally check pricing, ownership, source/reference mapping and summaries. New 130-offer integration coverage crosses two full batches and a short final batch; a forced failure in batch two rolls back the search header and the earlier batch. Empty successful searches skip batch insertion. All operations preserve original/selling snapshots and existing expiry behavior.

The response remains 23.3 MiB uncompressed. This optimization does not implement compression, pagination, admission limits or retention cleanup. The fresh database's offer table including indexes/TOAST was about 17.2 MiB for one search and 68.2 MiB for four; batch insertion does not reduce stored snapshots. Expired rows are not automatically deleted by these changes. Production capacity cannot be inferred from local replay: separately controlled staging/VPS load validation, HTTP compression and an agreed expired-search retention policy remain important before materially increasing traffic.

Private measurements, paired release binaries and comparison JSON: `.local/evidence/search-performance-20260910/`. Capture source: `.local/evidence/search-summary-20260910/`. These directories remain Git-ignored.

## Reproduce

Build `cargo build --locked --release --example search_load`. Use an EMPTY local PostgreSQL database whose name ends in `_load_test`; the example refuses non-local hosts and nonempty databases. Set `LOAD_DATABASE_URL`, `LOAD_CAPTURE_DIR` to a directory containing exactly one saved `roundtrip-all-{supplier}-search-*-raw.json` per supplier, and `LOAD_CONCURRENCY` to 1–8. Run the binary under your platform's RSS/CPU measurement tool. The capture dates must still satisfy Search date validation. Never point it at a working installation database.

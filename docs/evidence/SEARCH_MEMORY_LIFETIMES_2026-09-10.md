# Search memory lifetimes — local verification, 2026-09-10

Status: memory optimization verified locally. User subsequently authorized adding cleanup and then committing/pushing the combined change; see [cleanup evidence](SEARCH_CLEANUP_2026-09-10.md).

## Finding and change

Phase RSS sampling found that a single replay rose from approximately 252 MiB after supplier parsing to 450 MiB once all selling snapshots existed, and 465 MiB during SQL encoding. Both the complete original and complete selling trees were alive together. Comparison keys did not materially increase sampled RSS in this fixture; the samples do not measure exact live heap allocation.

Search now moves selected original offers into processing order. For at most 64 offers at a time, it encodes the immutable original snapshot as JSON, calculates/validates a small set of price changes, and reuses the original parsed tree for the selling response. Original JSON and reference maps are bound into the existing SQL batch and discarded after execution. This preserves original snapshots without keeping a second full parsed offer tree. Borrowed projection callers still receive a separate selling tree; both projection paths share the exact same calculation.

Every source offer is still validated before selection. The same winners, supplier priority, offer order, fields, exact numbers and reference mapping are retained. All batches remain inside one transaction. Summary aggregation now runs after batch inserts and before commit; a summary failure rolls back every inserted row. No incomplete response is returned. No schema, validity period, cache or cleanup policy changed.

## Validation

- `cargo fmt --check`, all-target clippy with warnings denied, 28 unit tests, 5 foundation tests and 6 production-fixture tests pass.
- Differential projection tests compare both borrowed and owned output/error results with the frozen original implementation for fixed/percentage pricing, precise amounts, missing/null and malformed fields. Only a small price-change plan is allocated before mutation.
- PostgreSQL 18 integration passes: all seven supplier subsets, reference ownership, partial failures, empty inventory, 130-row batches and later-batch SQL failure rollback.
- Integration now compares every stored original against the complete expected source offer and every stored selling value against the returned offer. An unknown `9007199254740993.00500` value remains precise, including its public numeric lexeme.
- A test-only nontransactional sequence proves 130 row inserts were attempted before a deliberately invalid summary. Search and offer counts remain unchanged afterward, demonstrating rollback after all three batches.
- No supplier network requests, booking, issuance or live database cleanup was performed.

## Final paired replay measurements

Same 2,184-source / 2,177-retained roundtrip capture as previous milestones, gzip enabled, three alternating runs per version at one and four concurrent requests, fresh disposable PostgreSQL databases. The baseline is the preceding deployed runtime (`e507ef9`) plus disabled-by-default phase instrumentation; final measurements use the current local implementation. Timed runs disable RSS sampling. `/usr/bin/time -l` measures the replay process peak RSS and CPU, excluding the separate PostgreSQL process.

| Concurrency | Metric | Before median | After median |
|---|---|---:|---:|
| 1 | Peak RSS MiB | 554.64 | 361.66 |
| 4 | Peak RSS MiB | 1,824.77 | 1,061.05 |
| 1 | Full-body HTTP ms | 1,019 | 936 |
| 4 | Full-body HTTP ms | 2,095 | 1,942.5 |
| 1 | Process CPU seconds/run | 0.92 | 0.85 |
| 4 | Process CPU seconds/run | 4.09 | 3.43 |
| 1 | Persistence phase ms | 813 | 751 |
| 4 | Persistence phase ms | 1,769 | 1,695.5 |

Peak process RSS fell about **35% / 42%**. Median full-body latency fell about **8% / 7%**; replay-process CPU about **8% / 16%**. These are small local Mac samples, not VPS capacity guarantees. The process includes mock fixture storage, client decompression and JSON verification, so its peak is not the standalone API's peak. Persistence timing includes projection, summaries, serialization and SQL; it is not SQL-only time.

Every request returned 2,177 offers and decoded to 24,446,992 bytes. Each four-request run persisted 8,708 offers. Normalized business fingerprints matched across all runs; original/reference correctness is additionally checked by integration tests because the normalized fingerprint excludes reference-named and timing fields. Gzip remains approximately 2.53 MiB, with random platform references causing small wire-size variation. No offer truncation or database snapshot size reduction is claimed.

## Diagnostic phase samples

One separate single-request profile per version, `LOAD_PROFILE_MEMORY=1`. Values are instantaneous process RSS (maximum of repeated samples for a phase), not per-phase heap allocation or an exhaustive peak. RSS includes allocator-retained pages. Before/after selling-ready samples occur at the completion of selling generation, which now interleaves with SQL batches.

| Phase | Before MiB | After MiB |
|---|---:|---:|
| Supplier inventories parsed | 251.80 | 251.86 |
| Comparison keys ready | 252.11 | 252.09 |
| All selling snapshots ready | 450.17 | 272.23 |
| SQL batch encoded, highest sample | 464.56 | 272.23 |
| Offers persisted, before commit | 464.56 | 272.30 |
| Wire body collected by replay client | 473.16 | 281.05 |
| Client JSON parsed | 520.16 | 331.58 |

An initial batch-release-only experiment reduced peak RSS just 2–3%. Its logs are retained under `initial-batch-release/`; the final implementation additionally reuses the owned offer tree, which accounts for the larger measured reduction. Final/raw logs, stage summaries, binaries and reproduction scripts are in `.local/evidence/search-memory-20260910/` and are not committed because captures can contain supplier data.

`examples/support/memory.rs` is an optional offline-only RSS subscriber using the dev dependency memory-stats. Runtime code emits debug phase names without payloads; the production INFO logger does not emit them or collect RSS. Profiling support does not add an API response field or a production memory sampler.

## Retention and release decisions

The user explicitly requires permission before commit or push. Their subsequent instruction authorizes this combined memory/cleanup release after cleanup is implemented and verified.

The user also questioned retaining temporary Search data beyond 15 minutes. General unused Search inventory can be a candidate for cleanup after 15 minutes; current usability remains 10 minutes. Selected offers are referenced by RePrice and booking rows, so a separate dependency-aware policy is required. It must address active operations and whether expired references should retain a lightweight marker for the current 410 behavior. A subsequent cleanup patch now implements that policy without extending the 10-minute validity period; see cleanup evidence.

This milestone’s disposable profiling and integration databases were removed after verification; local evidence files remain available.

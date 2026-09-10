# Lossless Search gzip — 2026-09-10

## Change

The application router adds tower-http 0.6.11 response compression, gzip only, `CompressionLevel::Fastest`, default content exclusions plus a 1,024-byte minimum. It negotiates Accept-Encoding, preserves identity fallback, and streams compression over the already serialized response. Offer selection, source/selling snapshots, numeric handling, markup and database transactions are unchanged. There is no request decompression or new migration.

This follows the user's authorization to proceed carefully after the [Tripfeels comparison](TRIPFEELS_OPTIMIZATION_COMPARISON_2026-09-10.md). The reference project was not modified.

## Verification

- Formatting and all-target clippy with warnings denied pass.
- 26 unit, 5 foundation and 6 production-fixture tests pass. Foundation tests compare exact decompressed OpenAPI bytes against identity; check absent/identity/unsupported/gzip-q-zero/weighted-gzip negotiation, Content-Encoding, Vary, Content-Length, no-store, request IDs and content type; verify small 200/401/503 responses stay unchanged.
- Disposable PostgreSQL 18 integration passes. Its Search calls now request gzip and decode before all existing assertions: all seven supplier subsets, pricing, reference ownership, partial failures, 130-offer batches and rollback on later-batch failure. Large successful Search responses must actually negotiate gzip.
- Release replay uses the real router, machine authentication, local captured supplier responses, projection, selection and PostgreSQL transaction. No supplier network calls, bookings or ticket issuance occur.

## Actual middleware measurements

Mac local environment from the [previous benchmark](SEARCH_PERFORMANCE_2026-09-10.md), three alternating runs per encoding at concurrency 1 and 4, each with a fresh empty PostgreSQL database. These are small local replay samples, not network or production capacity tests. Replay process RSS includes captured supplier fixtures and response decoding/verification; CPU includes replay/client work but excludes PostgreSQL. HTTP latency includes consuming the complete wire body, excludes subsequent client decoding and JSON assertions. Identity decode time measures the comparison buffer copy.

| Concurrent requests | Encoding | Median wire bytes/request | Median HTTP ms | Median decode ms | Median process CPU seconds/run | Median peak process RSS MiB |
|---|---|---:|---:|---:|---:|---:|
| 1 | identity | 24,446,992 | 1,106 | 2 | 0.97 | 532.4 |
| 1 | gzip | 2,655,624 | 1,119 | 21 | 1.01 | 538.4 |
| 4 | identity | 24,446,992 | 2,193.5 | 2 | 4.17 | 1,849.2 |
| 4 | gzip | 2,656,117 | 1,980 | 22 | 4.43 | 1,815.2 |

Every response decoded to 24,446,992 bytes and contained 2,177 offers. Each single Search persisted 2,177 rows; each four-request run persisted 8,708. All normalized business fingerprints matched across both encodings. The existing replay fingerprint intentionally excludes reference-named fields and timing; it supplements the reference/precision integration assertions and is not a claim of byte equality between separate Searches with fresh random UUIDs.

Wire reduction is about **89.1%**, from **23.31 MiB to 2.53 MiB**. Random UUIDs cause slight compressed-size variation. The previous Python gzip proof used a different encoder/level and is not this middleware's measured compression ratio. Full-body local timings show modest CPU overhead; the concurrency-4 timing difference is not evidence that compression speeds up backend computation. No decoded-memory, database-size or VPS throughput improvement is claimed.

Local raw logs, time output and aggregate results are under `.local/evidence/search-gzip-20260910/` (not committed; source captures may contain supplier data). `examples/search_load.rs` accepts `LOAD_ACCEPT_ENCODING=identity|gzip`; use a fresh local database ending `_load_test` for each run. All other guards remain in place.

## Remaining work

Remove validation-only/equivalence-key copies with exact behavior tests, decide expiry cleanup policy, then consider ownership/pricing/supplier-epoch-safe shared snapshots. This release does not implement those changes or finish the overall optimization requirement.

## Deployed verification

Application commit `fca1e85cf8aaa638122704c79dc26cfad758e78b` deployed through [GitHub Actions 34459503903](https://github.com/babuas25/shapontravels/actions/runs/34459503903). Rust/PostgreSQL/deployment-script checks, Ubuntu release build and deployment all succeeded. Activation logs confirm this exact SHA and successful live/ready checks at 2026-09-10 09:19 UTC.

Public HTTPS verification at `https://sendbox.shapontravels.com` confirmed OpenAPI identity body 21,913 bytes and gzip body 5,771 bytes, with byte-identical decompression (SHA-256 `d838db796a5aee1298e50a4d9b32a0674ee56cfe4eb4154ad6f31ea145029277`). Weighted gzip negotiation works; gzip;q=0 returns identity. Vary, no-store and request IDs survive the proxy. Live/ready return 200, and unauthorized Search/admin supplier requests return 401; these small responses stay uncompressed. These public checks did not invoke suppliers. Search size measurements above remain offline replay evidence.

The 13 disposable databases created for this milestone were removed after verification. Evidence files remain locally available.

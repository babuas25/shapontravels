# Search persistence profiling and smaller SQL batches — 2026-09-10

Status: local implementation, replay comparison and integration verification completed. Awaiting commit/push permission; not deployed.

## Finding and implementation

The previous `persistence_ms` measurement combined projection, SQL encoding/execution, summary work and commit. Added aggregate `projection_ms`, `sql_encode_ms`, `sql_execute_ms`, `commit_ms` and `sql_batches` to the existing successful-Search performance event. These contain durations/counts only. SQL execution time includes driver/network/server waiting and is not a measurement of PostgreSQL CPU alone. Other transaction/setup/summary work remains included in overall persistence time.

Compared 64-offer statements against 16-offer statements. Smaller batches substantially reduced sampled PostgreSQL resident memory in the captured workload, without a measured latency penalty. Search now uses 16 offers per INSERT, still within one transaction across the complete Search. The captured 2,177 retained offers require 137 statements instead of 35. This is a row bound, not a byte bound for arbitrarily large individual offers. More SQL round trips may cost more on a remote database than on the local Unix socket used here.

Original snapshots, selling values, reference maps, pricing, offer order and summary handling are unchanged. Errors after earlier batches or summary aggregation still roll back the entire Search. Admission remains four active/eight queued/two-second maximum wait. No migration, shared cache, retention or supplier policy change is introduced.

## Paired local replay

Both release binaries use the same new timing instrumentation; the only difference between them is 64 versus 16 rows per INSERT. Existing captured roundtrip responses contain 2,184 source offers and 2,177 retained offers. Suppliers are replaced by the offline adapter, which only parses local files. No supplier traffic or production writes occur.

Three alternating samples per version at concurrency one and four, twelve runs total, on local macOS. A separate PostgreSQL 18.3 cluster uses a private Unix socket, 32 MiB shared buffers, 1 MiB work memory and 16 connections. It restarts for every sample; each sample uses a newly created empty database. No CPU/RAM cgroup limits apply on this host. Builds/tests did not overlap measured runs; OS file caches and host scheduling are not controlled.

| Metric (median) | One request: 64 → 16 | Four concurrent: 64 → 16 |
| --- | ---: | ---: |
| Sampled sum of PostgreSQL process RSS | 180.42 → 123.73 MiB | 573.86 → 346.30 MiB |
| Largest sampled PostgreSQL process RSS | 120.02 → 63.23 MiB | 126.59 → 71.63 MiB |
| Replay-process peak RSS | 354.69 → 353.61 MiB | 1,080.14 → 1,155.30 MiB |
| HTTP full-body time | 909 → 887 ms | 1,537 → 1,338.5 ms |
| SQL execution time per Search | 572 → 557 ms | 1,048 → 848 ms |
| Overall persistence time per Search | 716 → 697 ms | 1,249.5 → 1,067.5 ms |
| Whole-wave time, including client validation | 1,166 → 1,147 ms | 2,353 → 2,159 ms |

At four concurrent requests, sampled summed PostgreSQL RSS fell approximately 39.7%, SQL execution time 19.1%, and HTTP time 12.9%. Replay-process peak RSS increased approximately 7.0%; this is **not** an application RAM-reduction claim. More overlap between response generation and client validation is a possible explanation, not a proven cause. The database-memory benefit motivates the smaller batches, especially given the prior isolated VPS PostgreSQL cap failure.

PostgreSQL RSS is sampled by `ps` approximately every 25 ms plus command overhead, covering the dedicated postmaster and its direct children. Summing RSS counts shared pages multiple times; it is not unique physical memory, a live-heap measure or the Linux cgroup peak previously reported. Sampling can miss peaks. `/usr/bin/time -l` separately measures the replay process, including fixtures, compression, client decoding and assertions. The two peaks occur at different times and must not be added into a claimed total-memory saving. No production/VPS capacity improvement is established by these local samples.

All 30 responses passed the existing stable business fingerprint and exact decoded length (24,446,992 bytes), and each returned/persisted 2,177 offers: 65,310 offers across the disposable databases. Fingerprints omit reference/timing fields; complete snapshot/reference assertions are covered separately by database tests. Median relation sizes remained about 17.35 MiB for one Search and 68.88/68.90 MiB for four; small differences in UUID compression/index allocation are not storage savings. Stored original JSONB column bytes were identical between variants (6,810,322 per Search). No database-growth or wire-size reduction is claimed.

## Verification and follow-up

- Release replay build, formatting, strict all-target Clippy and all 43 ordinary unit/foundation/fixture tests passed.
- Full disposable PostgreSQL integration passed in 36.98 seconds. Coverage includes complete original/selling snapshots, exact numeric lexemes, reference ownership, late-batch SQL failure, all-rows-inserted summary rollback, admission and cleanup. Its database was removed afterward.
- Reproduction runner, both binaries, raw logs, per-process RSS samples, aggregate column sizes and summaries are under ignored `.local/evidence/search-persistence-20260910/`. Run `python3 .local/evidence/search-persistence-20260910/measure.py` from the project root with its saved binaries and existing Search-summary captures. The script refuses nonzero replay outcomes and checks counts/size/fingerprint before recording a successful sample. It drops each successful sample database and stops the separate cluster, including on failure.
- Next capacity verification should repeat the constrained VPS comparison, including database cgroup memory and the final twelve-arrival admission policy. The prior four-success/eight-busy VPS result remains the latest VPS evidence. Do not increase concurrency based on these local results alone.
- User permission is required before committing/pushing this locally verified runtime change; deployment has not occurred.

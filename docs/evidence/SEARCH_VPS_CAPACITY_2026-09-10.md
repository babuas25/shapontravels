# Controlled VPS Search replay — 2026-09-10

## Scope and isolation

The user authorized the observation/capacity step, then explicitly approved committing/pushing the observer scripts and verification documentation after reviewing the results. This follow-up contains no application runtime change or deployment.

Target: existing Ubuntu x86_64 VPS, two vCPUs and 3,915 MiB RAM. Runtime source is commit `78cb33c`, whose application code is the deployed `b739a579fce5792b1ef4c80d57dbe434ccbbbad4`; the later commit only records evidence. Rust 1.98.1 builds the existing `search_load` release example with the committed lockfile and one build job.

Supplier responses are replayed from existing captured JSON through the real HTTP router, authentication, pricing, selection, SQL persistence and gzip response collection. The replay adapter cannot make supplier requests. Original response JSON substrings were transferred without numeric reserialization. Each sample uses a new empty database in a separate deploy-owned PostgreSQL 18 cluster on loopback port 55439, with no production credentials or business data.

Resource caps reserve capacity for the production API:

- Build: one CPU equivalent, 1,200 MiB cgroup memory, zero swap allowance.
- Replay: one CPU equivalent, 1,600 MiB cgroup memory, zero swap allowance.
- Isolated PostgreSQL: half a CPU equivalent, 512 MiB cgroup memory (final comparison), 32 MiB shared buffers, 1 MiB work memory and 16 connections.
- Readiness sampling every two seconds records public HTTPS status/latency, available RAM, free swap and production service state/memory. Each sample also checks readiness before/after and requires at least 1,900,000 KiB available memory before starting.

Three samples per concurrency level are run in order 1 → 2 → 4, reviewing each level before escalation. Every sample requires all 2,177 baseline offers per request to be returned and persisted, the same 24,446,992 decoded response bytes and the prior stable business fingerprint. That fingerprint excludes references/timing and is a regression signal, not a substitute for the existing full original/selling/reference/decimal integration tests.

## Interpretation limits

This is a constrained offline replay on the VPS, not an uncapped production capacity or supplier-latency test. `/usr/bin/time -v` peak RSS includes the replay fixtures and simulated client decoding/verification, and excludes the separate PostgreSQL process. HTTP time ends when the gzip body has been fully collected and excludes client decompression. Wall time includes client validation. Database relation size is measured before the disposable database is dropped; no Search cleanup worker runs in the router-only example.

The fixture has 2,184 source offers and 2,177 baseline retained offers. Correction from the original capture summary: 7,207 was the total across 12 scenarios, not a single Search; 2,177 was the largest returned response in that matrix. Results do not establish capacity for arbitrary larger responses, simultaneous cleanup backlog, long supplier requests, different passenger/route sizes or sustained traffic. No concurrency limit or queue has been selected or deployed solely from this fixture.

## Results

All nine final samples passed (three at each level), with 21 successful Search responses and 45,717 offers persisted across disposable databases. Every response retained all 2,177 baseline offers, the exact baseline decoded size and the same stable business fingerprint. Gzip wire size was about 2.53 MiB per response.

| Concurrent Search | Median HTTP full-body time | Median replay peak RSS | Median replay CPU seconds | Median wave wall time | Median offer-table size per wave |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 6.192 s | 664.55 MiB | 5.52 | 8.421 s | 17.36 MiB |
| 2 | 12.363 s | 827.48 MiB | 9.27 | 15.709 s | 34.53 MiB |
| 4 | 24.265 s | 1,404.84 MiB | 15.93 | 29.609 s | 68.75 MiB |

HTTP medians pool all requests at a level; other medians use the three wave samples. These are current-version measurements, not a before/after VPS speedup claim. The replay includes synthetic client work and is CPU-capped, so these times are not predicted live supplier Search latency.

An initial 256 MiB PostgreSQL cap passed levels 1 and 2 but caused an isolated database cgroup OOM kill on the first four-request attempt; test responses failed with database-unavailable 503. No production process was restarted. That failure was retained separately, the test database budget increased to 512 MiB, and **all three levels rerun** under the final equal conditions above. The final PostgreSQL cgroup peak reached 502.83 MiB with zero OOM/max events; four-request replay sampled cgroup peak reached 1,516.33 MiB. Cgroup totals include memory beyond process RSS and are not interchangeable with the table's RSS values. The database cap failure is not evidence that the entire 4 GB VPS cannot serve four requests.

Across 557 recorded health samples during preparation/testing, all returned 200/ready. Maximum sampled health latency was 937.1 ms and minimum available host RAM was 1,586.92 MiB. Sampling has session-transition gaps and does not establish uninterrupted availability between samples. Test cgroups disallowed swap; host free swap fell by 12 KiB, so host-wide zero swap use is not claimed.

## Completion and next work

The disposable PostgreSQL cluster and monitor were stopped; the entire VPS work directory, source/build toolchain, captured payloads and test databases were removed. Final HTTPS live/ready checks passed, the production PID remained 49952, and available RAM returned to about 3,452 MiB with disk usage 4.1 GB. The root-installed read-only observation helper remains available. Production observation still shows zero cleanup backlog and no Search inventory/deletion event.

The next optimization target is bounded expensive-Search admission with a finite queue/wait budget, plus profiling database persistence under concurrent runs of the largest captured response. Start by evaluating one/two active requests as candidates; these are **not validated production limits**. Keep every valid offer and atomic original/selling/reference persistence. Increasing concurrency alone lengthened latency markedly in this constrained replay, and the isolated PostgreSQL memory result shows that sizing only the API process is insufficient. Shared caches or changes to snapshot storage require separate ownership/reference/expiry validation.

Local raw logs, the failed 256 MiB attempt, health samples, runner scripts and numerical summary are in `.local/evidence/vps-capacity-20260910/`. No application runtime change or deployment was performed during measurement. The user subsequently approved committing/pushing the observer scripts and this evidence; `[skip ci]` avoids rebuilding/redeploying unchanged runtime code. Observation scripts pass bash syntax checks; documentation passes `git diff --check`. The deployed application code was built successfully with its committed lockfile and exercised by the nine replay samples.

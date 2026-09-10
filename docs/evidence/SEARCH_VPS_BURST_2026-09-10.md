# Controlled VPS twelve-arrival replay - 2026-09-10

## Scope and source identity

The user authorized isolated VPS validation of the local Search admission change before committing/pushing/deploying it. The application under test uses four active slots, eight queued slots and a two-second queue deadline. Production is not replaced by the test binary.

A minimal source tar contains current Cargo.toml/Cargo.lock, pinned Rust toolchain, source, migrations and examples, including the uncommitted admission module. It excludes environment secrets and private evidence. Local and uploaded tar SHA256 match: `77c17760d187165b5cf0fd739668fd3374da035d3f598861a29bb7023da51327`. A per-file manifest is retained in `.local/evidence/vps-burst-20260910/source-manifest.json`. The git base is d47c205 plus the locally verified admission/burst/queue changes; no commit is created merely for this test.

## Resource isolation and measurement

The VPS has two CPUs and about 3,915 MiB RAM. Build uses Rust 1.98.1, the committed dependency lockfile, release optimization and one cargo job. Its user-owned cgroup is limited to 1,200 MiB RAM, 150% CPU (1.5 CPU equivalents), and no swap allowance. The monitor samples production readiness, latency, available host RAM, swap and production service state every two seconds. This build budget leaves CPU headroom for the production API.

Replay is separately capped at one CPU equivalent, 1,600 MiB cgroup memory and no swap allowance. A private PostgreSQL 18 cluster on loopback port 55439 has a 512 MiB cgroup budget for the one-request smoke test, increased to 768 MiB before the twelve-arrival burst, half a CPU equivalent, 32 MiB shared buffers, 1 MiB work memory and sixteen connections. Production PostgreSQL credentials/database are not used. The prior four-request PostgreSQL peak was about 503 MiB, too close to a 512 MiB cap to leave headroom for twelve authentication connections. The burst database budget therefore increases deliberately within the host memory budget. Replay CPU/RAM caps otherwise match the earlier test. Results remain explicitly constrained, not uncapped production throughput.

First run a one-request smoke test, review health/results, then three waves of twelve simultaneous arrivals. No admission environment overrides are supplied: defaults must report 4 active / 8 queued / 2000 ms. The offline adapter reads the saved roundtrip supplier responses and cannot call supplier APIs. Each successful response must return 2,177 offers with the prior business fingerprint and 24,446,992 decoded bytes. Verified SEARCH_BUSY responses are recorded separately; any other failure aborts the run. Persisted offers must equal successful requests multiplied by 2,177.

Before each sample, readiness must succeed and host MemAvailable must be at least 1,900,000 KiB. Each sample uses an empty disposable database, removed after measurement, and checks public readiness afterward. A response-count success is not a guarantee of real supplier Search latency; replay omits supplier network delay. Peak process RSS includes fixtures and simulated-client decoding/assertions, excludes PostgreSQL, and differs from cgroup memory/cache totals. Health sampling does not establish uninterrupted availability between samples. Queue time is separate from total HTTP latency, which includes authentication, processing and body collection.


## Results

The release build completed successfully in 16m02s. One-request smoke passed: 1 success, 0 busy, 2,177 persisted offers, full-body latency 6.698 seconds and peak replay RSS 664.71 MiB. Production readiness was reviewed before starting the burst.

| Twelve-arrival wave | HTTP 200 | SEARCH_BUSY | Persisted offers |
| --- | ---: | ---: | ---: |
| 1 | 4 | 8 | 8,708 |
| 2 | 4 | 8 | 8,708 |
| 3 | 4 | 8 | 8,708 |

All thirteen successful responses including smoke retained every baseline offer, the prior business fingerprint and exact decoded size. Busy responses matched the expected 503 body and Retry-After: 1. No other replay errors occurred.

For the three burst waves, median successful full-body latency was **23.532 seconds**, median busy-response latency **3.099 seconds**, median complete-wave time **29.070 seconds**, and median replay process peak RSS **1,519.65 MiB**. Burst process CPU median was 16.33 seconds. Success HTTP times ranged 18.226-28.885 seconds; busy times ranged 2.579-4.057 seconds. The two-second setting is the admission timer, **not a two-second end-to-end HTTP response guarantee**. Total timings include authentication and executor scheduling/body work under CPU caps; the admission timer's isolated elapsed time was not instrumented, so a strict 2,000 ms wall-clock response deadline is not demonstrated.

Sampled cgroup high-water marks were 1,570.42 MiB for replay and 499.44 MiB for the isolated PostgreSQL cluster (different accounting from process RSS). Observed replay/PostgreSQL memory events showed zero max-limit events, OOM events and OOM kills. Across 551 health samples during preparation/testing, all returned 200/ready; maximum sampled health latency was 475.3 ms, minimum available host RAM 1,527.57 MiB, and observed free swap stayed at 131,056 KiB. This does not assert uninterrupted availability between samples.

## Interpretation and completion

The configured bound protects the test memory budget, but **does not satisfy an all-success twelve-arrival requirement** under these constrained VPS conditions: eight of twelve requests were rejected in each wave. The earlier local result (eight successes/four busy) must not be substituted for this VPS result. These are capped offline results, not an uncapped production rejection-rate forecast or supplier-latency prediction. More active slots must not be assumed safe solely from this run; no automatic limit increase was made.

The private cluster/monitor were stopped and the entire VPS test directory, captured responses, toolchain, build files and test databases were removed. Final HTTPS live/ready checks passed; production PID remained 49952, available RAM returned to about 3,445 MiB, and disk usage returned to 4.2 GB. Fixed read-only production observation still showed zero eligible cleanup backlog and zero Search inventory statistics/RePrice/booking counts.

Logs, source tar, per-file hashes, aggregate results and the hash-verified Linux replay binary are retained locally under `.local/evidence/vps-burst-20260910/`. Keeping the binary locally allows another comparison of this exact code without rebuilding on the VPS; no test payload remains on the VPS. Current local runtime source hashes still match the measured package. No new runtime code edits, commit, push, application deployment or live Nginx changes were made during this verification step.

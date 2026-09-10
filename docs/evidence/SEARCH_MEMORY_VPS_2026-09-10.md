# Controlled VPS comparison after memory optimizations — 2026-09-10

The user authorized continuing with isolated VPS capacity verification. No application code, production configuration, supplier settings or deployment changed during this test.

## Source identity and isolation

- Baseline: saved Linux replay binary from the prior admission verification; SHA256 `85b2980236cbeeb9bbb089da26f633f6ef6c7b41ab13b2e01dd7a1d9cc7063f1`. Its archived Search/admission/router source matches application commit `7c24ab0`.
- Candidate: current application `b08b34d`, packaged from documentation head `168e350`. Source tar SHA256 `88b2f53da6af8c8c10bcd3906082f471b4308a208dcc510a4b5f68be7750c450`; uploaded hash verified. Candidate Linux replay SHA256 `a2072d2184cea86866a4ddd71bbcab9c3a2902333377b18ac1c87148c4a3933b`.
- Candidate includes both 16-row persistence batches/timing and consuming response serialization. This comparison measures the combined release, not the isolated effect of either change.
- Rust 1.98.1 release build, locked dependencies, one build job, 1,200 MiB memory cap, 150% CPU quota and zero swap allowance. Build service completed in 16m32s. Replay measurements began after the build ended.
- Replay: 1,600 MiB memory cap, one CPU equivalent, no swap. PostgreSQL 18: separate deploy-owned cluster, loopback port 55439/private socket, 768 MiB cap, half a CPU equivalent, 32 MiB shared buffers, 1 MiB work memory and sixteen connections. These exact replay/database limits apply to both versions at both concurrency levels.
- One candidate smoke test, then three alternating samples per version at four arrivals and twelve arrivals. Each sample has a fresh empty disposable database; the isolated PostgreSQL service restarts between samples. Cluster files/OS cache and host scheduling are not reset to a fully cold state. Production database credentials/data are never used.
- Existing captured roundtrip supplier JSON is returned by the offline adapter; no supplier network/mutation capability exists in that adapter. Every successful response must contain 2,177 offers, 24,446,992 decoded bytes and the prior business fingerprint. Persisted offers must equal successful requests × 2,177. All variants use 4 active / 8 queued / 2,000 ms queue wait without overrides.

These are constrained same-host offline replays, not uncapped production throughput, supplier latency or maximum hardware capacity. Process RSS includes fixture storage, compression and simulated-client parsing/assertions. It excludes PostgreSQL and is not isolated deployed-API memory. Cgroup memory includes additional accounting such as cache; process and cgroup peaks are not interchangeable and need not be ordered identically. Do not add separately occurring process/database peaks into an exact combined saving.

## Accepted results

All four-request waves had four successes and zero busy responses. Every twelve-request wave had **four successes and eight `SEARCH_BUSY` responses**, for both versions. Each wave persisted 8,708 offers. The candidate smoke test had one success, 2,177 persisted offers, 6.407 seconds full-body latency and 660.95 MiB peak process RSS.

| Median metric | Four arrivals: baseline → candidate | Twelve arrivals: baseline → candidate |
| --- | ---: | ---: |
| Successful HTTP full-body latency | 24.110 → 25.945 s | 24.028 → 24.522 s |
| Busy HTTP latency | Not applicable | 3.804 → 3.495 s |
| Wave time including client validation | 29.619 → 33.436 s | 29.362 → 32.014 s |
| Replay process peak RSS | 1,527.57 → 1,495.94 MiB | 1,395.35 → 1,430.07 MiB |
| Replay cgroup peak memory | 1,523.81 → 1,495.88 MiB | 1,391.33 → 1,425.68 MiB |
| Isolated PostgreSQL cgroup peak memory | 409.39 → 209.34 MiB | 414.39 → 210.59 MiB |
| Replay process CPU per wave | 14.81 → 18.52 s | 15.76 → 17.58 s |

The database peak fell **48.9% / 49.2%**. Replay-process RSS changed **−2.1% / +2.5%**, so there is no consistent substantial replay-RAM improvement demonstrated on this VPS. Successful HTTP medians increased **7.6% / 2.1%**, replay CPU increased **25.1% / 11.5%**, and wave times increased **12.9% / 9.0%**. The combined release trades significantly lower PostgreSQL memory for extra replay CPU/time in these small capped samples. It is not a VPS speedup result. Attribution between smaller statements, response encoding, scheduling and simulated-client work requires a separate controlled comparison.

All 13 accepted runs (smoke plus twelve comparison waves) cover 97 requests: 49 successes and 48 expected busy responses. All 106,673 persisted offers passed count/fingerprint/decoded-size checks. Fingerprints exclude dynamic references/timing; complete original/selling/reference/precision correctness remains covered by the previously passing integration tests. Table sizes remain approximately 68.8 MiB per successful four-Search wave; no material storage saving is claimed.

Busy responses are whole-request 503 `SEARCH_BUSY`, with the replay validating `Retry-After: 1`. The configured two-second deadline is admission waiting time, not complete HTTP time: authentication and executor scheduling under CPU caps are also included in the measured busy latency. No all-success twelve-arrival claim follows from these results.

## Monitoring correction and safety evidence

Production readiness and available host RAM were sampled approximately every two seconds. Tests require a successful readiness check and at least 1,900,000 KiB MemAvailable before each sample. The monitor stops only test services on three successive readiness failures or available RAM below 1,000,000 KiB. Replay and PostgreSQL final cgroup `max`, `oom` and `oom_kill` counters must all be zero for a sample to pass.

The deploy user's systemd manager has `Linger=no`. After the four-request stage, an SSH-session gap stopped the monitor before the first twelve-request stage. Those six initial burst runs passed response/cgroup assertions but lacked continuous health sampling; they are preserved separately as `unmonitored-burst/` and **excluded from the table and accepted counts**. This is not represented as a production failure.

All six twelve-request samples were repeated with monitor and replay coordinated within the same SSH session, checking monitor active before/after every wave. The final summarizer checks each accepted sample against timestamped health coverage, including the first/last sample boundaries and absence of gaps greater than ten seconds. The monitor still has an explicit session gap covering the excluded preliminary burst; no uninterrupted overall monitoring claim is made.

Across the collected monitoring intervals, **685 samples were ready/200, zero failures**, maximum sampled latency 1,040.3 ms, and minimum available RAM 1,605,840 KiB (about 1,568 MiB). Free swap stayed at 131,056 KiB in observed samples. There were no guard aborts and every accepted replay/PostgreSQL sample had zero memory-limit/OOM events. Sampling does not establish behavior between observations.

## Cleanup and conclusion

The isolated cluster and monitor stopped; the full VPS test directory, toolchain/build files, captures and disposable databases were removed after hash-verified evidence download. Production PID remained **75311** throughout the before/after checks. Final HTTPS live/ready both passed, available RAM returned to about 3,450 MiB and disk usage to 4.2 GB. The read-only production observer still showed zero cleanup backlog and zero RePrice/booking rows at observation. No production restart, configuration adjustment, new migration or supplier call occurred.

Evidence, manifests, reproduction scripts, accepted/excluded raw logs, memory events, summaries and both the candidate binary and prior baseline reference are under ignored `.local/evidence/vps-memory-20260910/` and `.local/evidence/vps-burst-20260910/`. The candidate binary is retained locally so this exact release can be compared without another VPS build. Runtime/repository source was unchanged; only this report and requirement tracking are updated locally.

**Decision:** keep current admission limits. Lower PostgreSQL memory is verified, but twelve-arrival success and faster processing are not. The next justified optimization experiment is to isolate CPU cost and compare a middle batch size/response serialization independently under matched constraints. Shared caching still needs supplier-reference reuse/freshness/ownership validation; it cannot be assumed safe or claimed complete from this benchmark. Increasing active slots solely because PostgreSQL now has more memory headroom is not justified by the replay-process memory and unchanged rejection results.

Subsequent user direction: perform the further CPU/batch-size optimization and capacity testing when replacing the VPS. That work is deferred; current deployed code/settings remain unchanged. The user subsequently authorized this documentation-only commit/push, without another deployment.

# Twelve simultaneous Search arrivals - local verification

## User requirement and revised proposal

The user clarified that 10-12 customers may Search at the same time. The prior local draft of two active slots, four queued slots and a two-second wait was too restrictive for that burst. The final user-approved local policy is **four active Search responses, eight queued requests, and a maximum two-second queue wait**. The user explicitly rejected the intermediate 90-second proposal. Values remain configurable: active 1-8, queued 0-32, wait 1-2000 ms; larger waits are rejected. The replay example now accepts 1-12 concurrent arrivals.

Queue wait means waiting for an active slot before supplier work begins. It is not a mandatory delay and is not supplier execution time: an idle slot starts immediately, and a queued request starts as soon as a slot opens. Requests still waiting at the deadline receive 503 SEARCH_BUSY with Retry-After: 1. The bound counts requests, not distinct customers; twelve arrivals fit only when sufficient active/queue capacity is free. It does not guarantee all live supplier searches finish successfully.

## Tests and isolation

All 43 unit/foundation/fixture tests pass, with formatting and all-target clippy warnings denied. PostgreSQL integration passes in 40.09 seconds. A new deterministic burst test fills four active slots and eight waiting slots, rejects a thirteenth arrival, then drains the waiting requests in waves and checks all permits are restored. Existing timeout/cancellation, body completion/error/disconnect and authenticated no-supplier-call/no-inventory-insert overload tests also pass.

Three historical replay samples using the intermediate 90-second defaults without admission environment overrides successfully process twelve arrivals each. All 36 responses return/persist 2,177 offers with the baseline business fingerprint and 24,446,992 decoded bytes; each disposable database contains exactly 26,124 offers. These prove the local captured workload and configuration wiring, not a 90-second supplier delay or VPS traffic SLA. A shorter 25 ms deadline is used to test timeout behavior deterministically; no test needs to wait 90 seconds to validate the same timeout mechanism.

## Comparative measurement method

Compare four versus six active slots with twelve concurrent arrivals, eight queue slots and an explicit 30-second queue budget. The same current release binary and captured all-supplier roundtrip response are replayed three times per limit, alternating order. Every sample uses a new empty local PostgreSQL 18 database. All responses must preserve the baseline count, decoded size and stable business fingerprint.

Preliminary timings overlapped local builds/tests and are retained separately; the final comparison is rerun after build/tests complete with no concurrent agent-launched build/test workload. Final timing/RSS evidence is in `.local/evidence/search-burst-20260910/final/`; default-policy assertions are under `defaults/`. macOS process RSS includes fixtures and simulated client parsing/validation, excludes the separate PostgreSQL process, and is not VPS process memory or end-to-end live supplier latency. No full twelve-active memory comparison or production capacity guarantee is claimed.

## Final local results

| Active slots (twelve arrivals) | Median peak replay RSS | Median HTTP full-body time | Median complete-wave time |
| --- | ---: | ---: | ---: |
| 4 | 1,282.75 MiB | 3.164 s | 5.796 s |
| 6 | 1,805.28 MiB | 3.443 s | 6.155 s |

All 72 requests across six comparison samples returned and persisted every one of the 2,177 baseline offers, with identical baseline decoded size and normalized business fingerprint. Six active slots used about 40.7% more median replay RSS, while median full-wave time was about 6.2% longer in this local experiment. One six-slot sample was faster, so this is a median tradeoff rather than a claim that six always runs slower. Four active slots plus eight queued requests are retained as the local proposal for the specified twelve-arrival burst. Production throughput and supplier/network behavior still need VPS validation.

## Deployment prerequisite

Read-only inspection found `proxy_read_timeout 150s`. The earlier proposed increase to 240 seconds was intended for the now-rejected 90-second queue. It is withdrawn: retain the existing reviewed 150-second timeout. No live Nginx change is required solely for the two-second queue policy. Historical 30/90-second all-success replay results above are not a claim that twelve arrivals all succeed within the final two-second queue deadline. Those archived longer-budget experiments are no longer accepted by the final configuration validator.

No live supplier calls, production database writes, commit, push or deployment were performed for this follow-up. Disposable test databases are removed after each run. Further VPS twelve-arrival validation is required before describing this as measured production capacity.


## Final two-second policy: measured overload outcomes

The user finalized two seconds after the intermediate one-second correction. Default and hard upper bound are 2000 ms, with four active slots and eight queued slots. After formatting, all 43 unit/foundation/fixture tests, all-target clippy and release build passed, the final binary was replayed with twelve simultaneous arrivals, no admission setting overrides and `LOAD_ALLOW_BUSY=1`. No build/test workload ran concurrently with measurement.

Each of three local runs returned **8 HTTP 200 responses and 4 verified 503 SEARCH_BUSY responses**. Every successful response preserved all 2,177 offers, the prior fingerprint and exact decoded size. Each database contained 17,416 offers (8 x 2,177); busy responses were checked for the exact error and Retry-After: 1. All disposable databases were removed.

This supersedes any implication that all twelve arrivals succeed with the final short wait. It is a local captured-response result, not a production rejection-rate forecast. Evidence: `.local/evidence/search-burst-20260910/two-second/`. Production has not been updated and no commit/push was performed.

# Borrowed validation and equivalence serialization — 2026-09-10

## Change and preserved behavior

Search previously constructed and discarded a complete zero-markup offer for each source offer during validation. It now borrows that offer and runs the same pricing/coverage calculation without materializing the output tree. Optional discount fields and all existing validation errors are still checked; unsupported losing offers still fail before selection. Shape checks still run before a selling snapshot is cloned.

Equivalence keys previously cloned an offer, removed the approved reference/price/cabin fields and serialized the remaining tree. A structural borrowed serializer now produces the same exact key bytes directly. Field exclusions apply only at their original locations; identically named fields in unknown objects remain in the key. Exact serialized keys are still retained for comparison; this is not hash-only deduplication. Winner selection, supplier priority, cabin conflict handling and unknown-equivalence retention are unchanged.

Selling snapshots still require a separate owned value. Supplier-compatible response fields, numeric lexemes, references, gzip, database persistence and expiry policies are unchanged. No supplier network traffic, booking, issuance or migration was required.

## Verification

- Formatting and all-target clippy with warnings denied pass.
- 28 unit, 5 foundation and 6 production-fixture tests pass.
- Frozen pre-change implementations are compiled only for differential tests. Borrowed validation is compared to the previous zero-markup projection; fixed/percentage projection results and error kinds are also compared. Equivalence keys must match previous exact bytes and cabin data.
- Mutation cases cover missing/null/wrong-type fields, zero/negative/large precise numbers, optional discounts and charges, unknown nested fields with reserved-looking names, Unicode/escaping, and one-way/multicity production fixtures. Source JSON is checked for mutation. Existing tests retain RBD/brand distinctions, order-independent cabin conflicts, original-price selection, approved supplier ties.
- Disposable PostgreSQL 18 integration passes, including all seven supplier subsets, gzip-decoded prices/references, ownership, partial failures, 130-offer batches and later-batch failure rollback.

## Final replay measurements

Three alternating runs per version at concurrency 1 and 4, with the same captured roundtrip inventory, gzip enabled in both versions, and a fresh empty PostgreSQL database per run. Baseline runtime is the preceding gzip release (`fca1e85`); final modified release example is used for after measurements. Each replay contains 2,184 source offers and returns/persists the same 2,177 baseline offers. Four-request runs persist 8,708 offers. Every response decodes to 24,446,992 bytes; normalized business fingerprints match in all runs. Random platform UUIDs account for small gzip-size variation around 2.53 MiB.

| Concurrency | Metric | Before median | After median |
|---|---|---:|---:|
| 1 | Preparation ms | 93 | 20 |
| 4 | Preparation ms | 158 | 21 |
| 1 | Process CPU seconds/run | 1.04 | 0.94 |
| 4 | Process CPU seconds/run | 4.63 | 4.00 |
| 1 | Full-body HTTP ms | 1,132 | 1,088 |
| 4 | Full-body HTTP ms | 2,036 | 2,118 |
| 1 | Peak process RSS MiB | 527.75 | 543.11 |
| 4 | Peak process RSS MiB | 1,801.48 | 1,855.09 |
| 1 | Persistence phase ms | 829 | 864 |
| 4 | Persistence phase ms | 1,559 | 1,787 |

Preparation is about 78%/87% faster and total replay-process CPU about 10%/14% lower. **Peak RSS did not improve:** it was about 3% higher in these samples. Full-body latency improved about 4% at concurrency 1 but worsened about 4% at concurrency 4; the persistence phase dominated and varied. This change should not be represented as demonstrated RAM savings, uniformly faster Search or higher VPS capacity.

The initial review measurements likewise showed a large preparation/CPU improvement but mixed RSS, with better end-to-end latency in that run set. The table deliberately reports the final code's complete rerun, not a favorable subset. Initial logs are retained under `initial-review/` alongside final evidence in `.local/evidence/search-copies-20260910/`.

These small local Mac replay samples include fixture storage, client decoding and JSON verification in process memory/CPU; PostgreSQL runs separately and its CPU is excluded. HTTP timing consumes the whole compressed body before client decoding. Preparation includes coverage checks and equivalence selection; persistence includes pricing, summaries, serialization and database activity, not only SQL. Normalized fingerprints exclude reference-named/timing fields and complement the independent exact-key/projection/reference tests; separate Search responses have fresh UUIDs.

## Remaining optimization

This removes two kinds of repeated full-offer copying, primarily reducing preparation CPU. Full source/selling inventories, serialized comparison keys and database snapshots still exist. Peak-memory reduction needs further profiling of those lifetimes; expiry cleanup policy, client/pricing/supplier-epoch-safe sharing and production-equivalent capacity checks remain open. No additional offer filtering or truncation is introduced or proposed by this change.

## Deployment evidence

Application commit `e507ef9046ff01d0edcd73153fc02806107982c2` deployed successfully through [GitHub Actions 34461353085](https://github.com/babuas25/shapontravels/actions/runs/34461353085). All check/build/deploy jobs passed. Activation logs confirm that exact SHA and successful live/ready checks at 2026-09-10 09:41 UTC; the systemd service is active.

After activation, eight public HTTPS checks passed at `https://sendbox.shapontravels.com`: live/ready 200, unauthorized Search/admin supplier routes 401, identity/gzip/weighted-gzip/gzip-q-zero negotiation, matching decoded OpenAPI bytes, no-store and request IDs. OpenAPI remains 21,913 decoded bytes (5,771 gzip bytes), SHA-256 `d838db796a5aee1298e50a4d9b32a0674ee56cfe4eb4154ad6f31ea145029277`. These checks did not call suppliers; Search correctness/performance evidence above is local replay and integration evidence.

This milestone's disposable test databases were removed; final and initial-review logs remain locally available.

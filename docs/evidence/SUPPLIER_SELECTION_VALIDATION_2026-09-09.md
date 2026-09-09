# Supplier-total selection validation — 2026-09-09

Scope: production Login/Search/FareRules/RePrice through the current public Rust router, using disposable local PostgreSQL databases and temporary local identities/markup. No deployed production client, rule, supplier setting or database was changed by these tests. No Book, Cancel, PNR or ticket-issue call was made. Local RePrice acceptance only writes the disposable test database.

Request: DAC–CXB, 30 September 2026, one adult, Economy request, no carrier filter. Configured account currency BDT. Fixed BDT 500 markup. The final run used the user's newly activated production configuration, with booking and ticketing flags forced false only in the test process. The user's `.env` was not rewritten.

## Final successful sample

| Supplier | Raw Search offers | Retained offers | Selected-sample FareRules | Selected-sample RePrice |
| --- | ---: | ---: | --- | --- |
| Firsttrip | 29 | 10 | 200 | 200 |
| Takeoff | 26 | 26 | 200 | 200 |
| Triplover | 29 | 9 | 200 | 200 |

Search returned 45 offers from 84 raw offers, with `X-Search-Partial: false`. Selection removed 39 cross-supplier duplicates in 20 comparison groups. Fifteen retained offers had null cabin labels. Unknown or different non-price attributes remain conservative separation boundaries.

A separate Python Decimal audit checked every retained original against the captured same-call supplier bodies, rebuilt comparison groups, and independently verified original-total winners with Takeoff → Firsttrip → Triplover ties. It checked every passenger selling total, signed discount and aggregated selling total after fixed 500 markup. All assertions passed. Original supplier references can be reused across different VQ fare classes, so audit identity includes the complete original offer, not only transaction/item strings.

For one retained sample per represented supplier, public FareRules, RePrice and local acceptance returned 200. Stored source ownership/references and a single fixed-500 markup at the successful RePrice revisions were verified. Triplover sampling prefers a retained BG fare because of the separately observed VQ limitation below; other suppliers use their first retained fare. This is representative servicing evidence, not every-offer validation.

## Failed samples retained as evidence

1. Initial all-supplier run: Triplover's first retained VQ offer returned public FareRules 502. A direct read with the same saved original supplier references also returned supplier `isSuccess=false`; direct RePrice succeeded. A selected BG fare from that same Triplover capture returned success for both direct reads. This establishes an upstream FareRules business failure for the sampled VQ offer, rather than proving that the selection router misrouted it. It does not establish the supplier's underlying root cause or support suppressing the error.
2. A subsequent BG-preferred sample returned public RePrice 502 for Takeoff. Its underlying reason was not captured/root-caused. The final run's first retained Takeoff sample passed. Do not treat that pass as proof that every Takeoff BG fare is serviceable.

No failure was converted to success, invalid offers were not silently excluded, and no booking fallback or supplier mutation retry was performed. Existing pricing-coverage, complex-scope, branded-fare and aggregate-summary limitations remain documented in `docs/SEARCH_API.md`.

## Reproduction and private evidence

`tests/live_search.rs` is explicitly opt-in, requires an empty localhost database ending `_live_search_test`, and uses real configured supplier READS. `SELECTION_EVIDENCE_DIR` can capture private original/selected Search snapshots and supplier read request/response pairs in a fresh `.local/evidence/` directory with restricted permissions. `examples/selection_read_probe.rs` performs explicit read-only Triplover VQ/BG diagnostics from one saved audit.

Private evidence directories (Git-ignored): `.local/evidence/supplier-selection-20260909`, `.local/evidence/supplier-selection-bg-20260909`, `.local/evidence/supplier-selection-final-20260909`. Final independent audit: `selection-verification.json` and `verify.py` in the final directory. Do not publish raw supplier references or private snapshots.

This report records pre-deployment supplier validation. CI/CD and deployed-site smoke results must be checked separately.

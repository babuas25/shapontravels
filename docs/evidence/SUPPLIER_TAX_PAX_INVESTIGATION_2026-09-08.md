# Supplier tax aggregation investigation — 2026-09-08

## Conclusion

Fresh production Search evidence contradicts the narrower initial hypothesis that this is limited to Triplover MH/TK. The observed tax aggregation defect also occurs on Triplover EK and Firsttrip SQ. It is already present in raw upstream responses before our markup or public Search projection runs. The pattern is associated with specific `avlSrc` markers and passenger representation, rather than every offer of an airline.

Across **37 successful Search calls and 9,628 offer observations**, **256** have both component and top-level tax mismatches. Every one of those 256 exactly matches this defective aggregation formula:

```text
reported aggregate tax = passengerFares.adt.taxes
                       + passengerFares.chd.taxes
                       + passengerFares.inf.taxes
                         (missing/null types contribute zero)
```

The correct count aggregation for the observed requests is:

```text
sum(passengerFares[type].taxes * passengerCounts[type])
```

The observed result omits count multiplication and omits `cnn`. This is strong behavioral evidence of a supplier-side field mapping/aggregation defect. We do not have supplier implementation code, so the exact implementation, ownership of any shared upstream connector, and the commercial identity behind a color-valued `avlSrc` remain unconfirmed. The color is treated only as a grouping marker, not a verified provider ID.

## Scope and execution

- User authorized Search investigation for adult/child/infant combinations, not B2B/B2C account types.
- All three configured production supplier adapters were used directly. No database connection, public router, markup application, booking, cancellation, ticketing, FareRules or RePrice calls were made. Login/token refresh and bounded Search read retries are possible within the adapter.
- No pricing implementation or active supplier/client/markup settings were changed during this investigation. Added only a diagnostic example and this report; private analysis and captures are under `.local/evidence/tax-investigation-20260908`.
- The adapter used the **64 MiB Search cap left from the earlier interrupted work**; this investigation did not change it. All requested bodies completed. This is not a public API pricing pass or load/performance certification.
- One-way: DAC–SIN, 2026-10-15. Round-trip adds SIN–DAC, 2026-10-20. Multicity: DAC–SIN 2026-10-15; KUL–DAC 2026-10-20; DAC–MLE 2026-10-25.
- Cabin class 1, preferred and prohibited carrier lists empty for every request. Thus other returned airlines serve as controls; MH/TK were not exclusively filtered.
- Each supplier: seven one-way combinations, plus one-adult and five-passenger combinations for round-trip and multicity (11 calls each). Four additional Triplover one-way calls isolate child age/type and repeated-child counts (37 calls total).
- Results are repeated observations across requests, not 9,628 unique itineraries. Live inventory/source availability varies between requests.

## Supplier and airline findings

| Supplier | Calls | Offers examined | Tax mismatches | Affected carrier / marker |
|---|---:|---:|---:|---|
| Firsttrip | 11 | 2,328 | 2 | SQ / `#27d32a`, five-passenger round-trip |
| Takeoff | 11 | 2,427 | 0 | None observed |
| Triplover | 15 | 4,873 | 254 | MH 115, TK 46, EK 93; all `#ce25e4` |
| **Total** | **37** | **9,628** | **256** | |

Returned plating carriers: 6E, AI, BG, BS, CA, CZ, EK, EY, MH, MU, OD, QR, SQ, SV, TG, TK, UL (17 distinct values). No other aggregate tax mismatch was observed. This does not certify airlines, sources, routes or passenger combinations not returned in these calls.

The same MH/TK airlines also returned matching aggregate taxes through other markers. Triplover's `#ce25e4` returned correct taxes for one adult and other cases with one of each included type; it failed when repeated counts or `cnn` needed to be represented. On the round-trip comparison, this marker returned EK/TK rather than MH. Firsttrip's offending SQ marker was only observed in the five-passenger round-trip response, so there is no same-marker one-adult SQ control from this run.

## Passenger isolation: Triplover one-way

The following counts are **only MH/TK offers under `#ce25e4`**. Other returned offers are included in the full matrix below.

| Request | MH mismatched / observed | TK mismatched / observed | Interpretation |
|---|---:|---:|---|
| 1 adult | 0 / 16 | 0 / 5 | Baseline matches |
| 2 adults | 16 / 16 | 4 / 4 | Second adult tax omitted |
| 1 adult + 1 child, age 6 | 0 / 17 | 0 / 4 | Single `chd` included |
| 1 adult + 1 child, age 11 | 0 / 15 | 0 / 4 | Single `chd` included |
| 1 adult + 1 child, age 3 | 17 / 17 | 4 / 4 | Child represented as `cnn`; entire child tax omitted |
| 1 adult + 2 children, ages 6/6 | 16 / 16 | 4 / 4 | `chd` count 2; only one child tax included |
| 1 adult + 2 children, ages 3/11 | 16 / 16 | 4 / 4 | `chd` included, `cnn` omitted |
| 1 adult + 1 infant | 0 / 17 | 0 / 5 | Single infant included |
| 2 adults + 2 infants | 16 / 16 | 4 / 4 | Second adult and second infant tax omitted |
| 1 adult + 1 child age 6 + 1 infant | 0 / 16 | 0 / 4 | One of each included type matches |
| 2 adults + children ages 3/11 + 1 infant | 16 / 16 | 4 / 4 | Adult count and `cnn` omissions combine |

Round-trip `#ce25e4`: one adult gives 0/93 EK and 0/6 TK mismatches; five passengers give 93/93 EK and 6/6 TK mismatches. Multicity `#ce25e4`: one adult gives 0/18 MH and 0/16 TK mismatches; five passengers give 18/18 MH and 16/16 TK mismatches.

## Concrete numeric examples

**Triplover MH, one adult + one three-year-old child:** returned counts are `adt=1, chd=0, cnn=1`. Adult tax 15,534; CNN tax 13,534. Correct aggregate is **29,068**; component and top-level taxes are **15,534**, omitting CNN entirely. This isolates a type omission even without a repeated passenger count.

**Triplover MH, two adults + children ages 3/11 + one infant:** correct tax is `15,534 × 2 + 13,534 + 13,534 + 2,846 = 60,982`. Reported tax is `15,534 + 13,534 + 2,846 = 31,914`. Difference: **29,068**.

**Triplover EK, five-passenger round-trip:** correct tax is `19,994 × 2 + 17,994 + 17,994 + 2,846 = 78,822`. Reported: **40,834**. Difference: **37,988**.

**Firsttrip SQ, five-passenger round-trip:** correct tax is `16,288 × 2 + 14,288 + 14,288 + 2,846 = 63,998`. Reported: **33,422**. Difference: **30,576**.

## What was checked independently

Python Decimal arithmetic examined the raw response from each call, before any selling projection:

- All 9,628 offers' passenger counts match the requested adult, combined child and infant counts.
- Component base, total and AIT, and offer total, match count-weighted passenger values across all 9,628 observations.
- Every mismatch affects both `bookingComponents[0].taxes` and top-level `taxes`; both equal the defective formula above.
- In all 256 affected offers, component discount also matches count-weighted passenger discount. Using the correct weighted tax restores the original supplier equation `base + taxes + AIT + signed supplier discount = total`. This describes these raw signed supplier discounts, not our selling discount convention.
- Some passenger tax breakdown arrays are absent/empty despite nonzero tax. They were recorded as missing detail, not summed as proof of zero tax. This does not explain the aggregate count/type omissions.
- A separate raw component-discount aggregation difference was observed in 717 Takeoff offers. It is outside this tax investigation; the claim of zero Takeoff mismatches is specifically about aggregate tax, not universal price-field correctness.

Our current guard compares component tax against weighted passenger tax in `src/projection.rs`; public Search maps its rejection to `SUPPLIER_PRICING_COVERAGE_UNSUPPORTED`. This explains why the upstream defect can block the entire public Search. The direct adapter results in this report do not establish that public Search would return these offers.

## Full request matrix

A = adult, C = child, I = infant. `a1c1` means child age 6; `a1c2` and `a2c2i1` use ages 3/11. Explicit age labels identify the extra controls. All rows returned a supplier success status and a complete body.

| Supplier | Trip | Combination | Offers | Tax mismatches |
|---|---|---|---:|---:|
| firsttrip | multicity | a1 | 57 | 0 |
| firsttrip | multicity | a2c2i1 | 70 | 0 |
| firsttrip | oneway | a1 | 171 | 0 |
| firsttrip | oneway | a1c1 | 169 | 0 |
| firsttrip | oneway | a1c2 | 166 | 0 |
| firsttrip | oneway | a1i1 | 171 | 0 |
| firsttrip | oneway | a2 | 168 | 0 |
| firsttrip | oneway | a2c2i1 | 166 | 0 |
| firsttrip | oneway | a2i2 | 166 | 0 |
| firsttrip | roundtrip | a1 | 524 | 0 |
| firsttrip | roundtrip | a2c2i1 | 500 | 2 |
| takeoff | multicity | a1 | 207 | 0 |
| takeoff | multicity | a2c2i1 | 191 | 0 |
| takeoff | oneway | a1 | 147 | 0 |
| takeoff | oneway | a1c1 | 143 | 0 |
| takeoff | oneway | a1c2 | 141 | 0 |
| takeoff | oneway | a1i1 | 145 | 0 |
| takeoff | oneway | a2 | 145 | 0 |
| takeoff | oneway | a2c2i1 | 136 | 0 |
| takeoff | oneway | a2i2 | 140 | 0 |
| takeoff | roundtrip | a1 | 518 | 0 |
| takeoff | roundtrip | a2c2i1 | 514 | 0 |
| triplover | multicity | a1 | 200 | 0 |
| triplover | multicity | a2c2i1 | 350 | 34 |
| triplover | oneway | a1 | 244 | 0 |
| triplover | oneway | a1c1 | 248 | 0 |
| triplover | oneway | a1c1age11 | 218 | 0 |
| triplover | oneway | a1c1age3 | 254 | 21 |
| triplover | oneway | a1c1i1 | 244 | 0 |
| triplover | oneway | a1c2 | 250 | 20 |
| triplover | oneway | a1c2age6 | 244 | 20 |
| triplover | oneway | a1i1 | 243 | 0 |
| triplover | oneway | a2 | 242 | 20 |
| triplover | oneway | a2c2i1 | 251 | 20 |
| triplover | oneway | a2i2 | 239 | 20 |
| triplover | roundtrip | a1 | 792 | 0 |
| triplover | roundtrip | a2c2i1 | 854 | 99 |

## Reproduction and evidence

Diagnostic: `examples/tax_investigation.rs`. Explicit invocation: `cargo run --example tax_investigation -- --production-search-only`; extra child controls add `--follow-up`. It writes with create-new semantics, so reruns must use a fresh evidence directory rather than overwrite this run. Diagnostic-specific Clippy passed. The process completed successfully for both phases.

Private evidence contains 37 request bodies and their matching 37 raw response bodies, run logs, `analyze.py`, sanitized `summary.json`, and SHA-256 `manifest.json`. Raw captures use mode 0600 inside a mode 0700 directory and contain supplier references, so they are not committed or embedded in this report. No authentication tokens/passwords are included in the report.

The supplier-facing question is concrete: why do the affected mappings aggregate only ADT + CHD + INF once, omit CNN, yet correctly count passengers for base, total and discount? Suppliers must confirm which mapping or upstream integration owns this behavior before attributing it to an airline or naming an underlying provider. No supplier message was sent and no pricing correction was implemented.

# BG multicity Hold — 2026-09-10

User explicitly requested booking after reviewing the three-route all-airline Search. Used a fresh BG-filtered UAT Search and the lowest eligible refundable/bookable offer, with the previously supplied private adult/child details. The diagnostic bounds the BG Search selling fare to the previously shown 24,228 supplier fare plus the isolated test's 1,000 markup; no higher-priced replacement is silently selected.

| Flight | Date | Departure | Arrival | RBD |
|---|---|---|---|---|
| BG611 DAC→CGP | 23 September 2026 | 07:45 | 08:35 | G |
| BG126 CGP→DAC | 24 September 2026 | 07:45 | 08:40 | G |
| BG433 DAC→CXB | 25 September 2026 | 10:15 | 11:30 | G |

Times are returned flight-local strings, not converted UTC times. Fresh public Search returned four offers; first candidate RePrice HTTP 200, bookable/refundable true, unchanged price. Public local acceptance succeeded. Exactly one supplier Book returned Created with PNR; public Book HTTP 200 and persisted state held. Identical same-key replay sent no additional supplier Book.

Public PNR and Admin recheck each returned HTTP 200. Supplier PNR status is **Created**, not Booked, and latest lastTicketTime is empty. Thus retrieval is evidenced but ticketing readiness is not: current reconciliation semantics still flag non-Booked status for review, and no usable deadline/timezone authority is established. No Issue, Direct Issue or Cancel was called.

Independent Decimal audit: supplier RePrice/Book passenger and component fares match; original 24,228.00 BDT, accepted selling 25,228.00 BDT for 1 ADT + 1 child. Fixed 500 applied once per passenger, no per-leg markup. Book's existing component selling total matches the accepted total. No passenger names, PNR or supplier references are copied into this public repository report.

Private evidence: `.local/evidence/uat-bg-multicity-hold-20260910/`; retained isolated DB `shapon_bg_multicity_uat_booking_test`, with private `isolated-database.dump` backup. Archive listing verified, full restore not tested. Exact UAT hosts and BG-only segment checks enforced; no production supplier/database writes. Harness formatting, targeted strict Clippy and diff checks pass. No commit/push/deploy.

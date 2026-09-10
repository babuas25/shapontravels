# BS domestic UAT Hold and PNR verification — 2026-09-10

User requested BS domestic one-way, return and multicity testing, and prohibited Robot/AI/System passenger names. Reused the previously supplied private UAT passenger details (one adult and one child aged 3); confirmed those prohibited name tokens were absent. No names or travel documents are included here.

Exact Triplover UAT hosts were asserted. Searches requested BS, and both Search/RePrice hold candidates had to be exclusively BS across all flight segments, refundable and bookable, with unchanged RePrice price. Each independent case permits at most one supplier Book, with a durable create-new intent file and same-key replay checks. No Issue, Cancel or automatic mutation retry was sent.

| Case | Dates | Search offers | Supplier Book | Public Book | PNR / Admin recheck |
|---|---|---:|---|---|---|
| DAC→CGP one-way | 5 Nov 2026 | 7 | Created, PNR returned | 202 outcome_unknown before cabin fix | Both 200, Booked |
| DAC→CXB→DAC return | 10 / 13 Nov 2026 | 64 | Created, PNR returned | 200 held after cabin fix | Both 200, Booked |
| DAC→JSR, JSR→DAC, DAC→SPD | 17 / 20 / 22 Nov 2026 | 0 | Not called | Search 503 | Not called |
| DAC→CGP, DAC→RJH (two-route open-jaw alternative) | 25 / 28 Nov 2026 | 0 | Not called | Search 503 | Not called |

First multicity supplier error: no availability for the given criteria. Alternative multicity error: no eligible fare for the given availabilities. These are sampled inventory failures, not proof that BS multicity is categorically unsupported. Both return `ALL_SUPPLIERS_FAILED` publicly because only Triplover is active. One-way/return Search status arrays each contain a successful BS result.

Exactly **two supplier Book calls** in total. Both successful cases replayed the same public request/key without another dispatch. No repeat of the earlier international duplicate attempt occurred.

## Compatibility finding and fix

Domestic one-way Book preserved the selected flight, RBD, passengers and all monetary values, but returned cabinClass=null where RePrice said Economy. The exact cabin comparison caused the public unknown outcome. The validator now allows missing/null/empty cabin labels while preserving the supplier shape, rejecting conflicting explicit cabin labels and any flight/RBD change. Full monetary checks remain. The original one-way record stays outcome_unknown with verified Booked reconciliation evidence; it was not silently rewritten or manually resolved. Return was executed after the fix and passed the complete public Hold/PNR flow.

Local verification: 52 ordinary tests pass (40 unit, 6 foundation, 6 fixture), full disposable PostgreSQL suite passes in 40.72 seconds, strict all-target Clippy/formatting/whitespace checks pass. New tests cover missing cabin, explicit cabin conflict and changed flight/RBD. Synthetic test database removed.

## Prices and deadlines

Independent Decimal audit confirms unchanged supplier passenger/component fares between RePrice and Book and exactly one fixed 500 markup per passenger:

- One-way: supplier 8,164.00 → selling 9,164.00 BDT.
- Return: supplier 17,382.00 → selling 18,382.00 BDT.

PNR and Admin recheck both report Booked. Raw latest deadlines: one-way `09/12/2026 22:42:00`; return `09/12/2026 22:45:42`. Supplier timezone remains unverified; these are raw strings, not a UTC conversion or authorization to issue.

## Private state retained for continuation

- `.local/evidence/uat-domestic-bs-oneway-20260910/` and isolated DB `shapon_bs_oneway_uat_booking_test`.
- `.local/evidence/uat-domestic-bs-return-20260910/` and isolated DB `shapon_bs_return_uat_booking_test`.
- Each successful case has `isolated-database.dump` (0600); archive listings verified, full restore not tested. SHA256 respectively `0e577aa655165a90c6f68ae3470a9699cd44190201b496de4e969fa8acc55ba9` and `e754941519752c8f8167b6c270210e8a3d5cfd0eab4f2d813c3a08c6b81066c3`.
- Failed multicity evidence remains in matching `uat-domestic-bs-multicity[-alt]-20260910` directories; their disposable databases were removed because no Book was sent.

Raw passenger/reference/supplier data stays private (directories 0700, files 0600). Do not rerun these evidence directories or manufacture a new key to retry a booking. The generic harness's old international default remains unchanged and protected by its existing evidence guard. No working/production database update, production supplier traffic, commit, push or deployment.

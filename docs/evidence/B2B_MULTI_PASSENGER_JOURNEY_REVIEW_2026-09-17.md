# B2B multi-passenger and journey pricing review — 17 September 2026

The user requested verification that the corrected discount formula also handles multiple passengers, mixed passenger types, return and multicity itineraries.

Added frozen independent decimal expectations in `tests/fixtures/b2b-journey-discounts.json` and a regression matrix in `tests/tier_discount_matrix.rs`. Captured return and multicity fares each contain 2 adults, 1 child and 1 infant. The matrix covers fixed BDT 500 and percentage 1% markup at 55%, 60%, 85% and 100% shares (16 scenarios), asserting each passenger's gross/discount/payable and all aggregate amounts. Originals remain unchanged. A separate synthetic five-type fixture (ADT, CHD, CNN, INF, INS with unequal counts/prices) verifies 55/85/100% shares and rounding.

Expanded the disposable database Search/RePrice integration for return and multicity to cover both markup types and mixed first-segment airline selection (8 scenarios). Exact discount snapshots are checked against the independent expected amounts, and Search/RePrice snapshots must match, in addition to the existing accepted-price checks. Competing later-route and plating-airline rules verify that one first-route/first-segment rule applies to the whole-journey passenger fare, without stacking.

Frontend boundary regression now verifies all five passenger types, unequal counts, row totals, total discount, total payable and separately retained AIT. No double multiplication or adult-fare substitution was found.

Pricing remains per passenger: round supplier plus markup, calculate and round that passenger's tier discount, derive payable, then multiply by count and sum. Whole-journey fare means markup is not repeated per segment or direction. Search and RePrice share the same engine. Negative discounts are retained where markup pushes a supplier fare above gross.

Coverage limitation: these captured fares use one booking component for the entire journey. Unknown multiple-component or extra-service allocation remains explicitly unsupported by projection validation; it can return a pricing coverage error rather than invent an allocation. This review does not claim real supplier availability/response coverage for every passenger/itinerary combination.

No pricing runtime change was needed. Only regression tests and review evidence changed. No supplier Book, Issue, Cancel, live RePrice or acceptance was executed; integration supplier calls and acceptance are simulated in a disposable database. No working database settings, restart or deployment was performed.

Validation passed: both new matrix tests, expanded full disposable PostgreSQL suite (46.34 seconds), frontend prebooking boundary regressions, strict Clippy for the matrix/database targets, and whitespace checks.

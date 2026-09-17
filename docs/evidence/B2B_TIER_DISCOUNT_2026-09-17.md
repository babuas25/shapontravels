# B2B tier discount correction — 17 September 2026

The user clarified that markup increases supplier payable, and confirmed that tier percentages share the discount remaining between published gross and that marked-up supplier fare. This supersedes previous same-day interpretations of a tier share of markup or of gross minus unmarked supplier fare.

Per passenger: marked-up supplier total = half-up rounded supplier original totalPrice plus the resolved fixed/percentage markup. Available discount = published base + taxes gross minus marked-up supplier total. Agent discount = available discount times configured tier share, rounded to two decimals. Agent payable = gross minus agent discount. Rounded passenger amounts are multiplied by passenger counts. At 100%, payable equals marked-up supplier total. Original AIT remains included once in supplier cost and is not added to published gross.

The existing snapshot/API `commission` field retains its name for compatibility but represents the signed agent discount. UI labels now say Discount in desktop/mobile Fare tables and RePrice review. Negative adjustments are preserved when marked-up supplier fare exceeds gross; wallet money parsing remains unsigned, with only the reconciliation discount supporting a sign. No stored historical snapshot is rewritten. Main flight cards still display large Gross fare, with the arrow revealing Agent fare or staff Supplier fare.

Verification:

- 70 Rust library tests passed, one private recovery test ignored; six foundation and six production-fixture tests passed.
- Full isolated PostgreSQL integration passed in 46.75 seconds, including Search/RePrice fixed-markup discount checks, policy changes, snapshot stability and simulated wallet reconciliation.
- Strict Clippy, frontend prebooking regressions, TypeScript, targeted ESLint and whitespace checks passed.
- Unit regressions cover the user's 1% example at shares 55/85/100, multi-passenger rounding, separate AIT, fixed and zero markup, and signed discounts. A 5% follow-up verifies that increasing markup increases Enterprise payable and reduces discount.
- Rebuilt and restarted the local backend; readiness returned 200. Real Search through canonical B2B sessions returned three retained offers for DAC–JSR on 30 September, one adult. No Book, Issue, Cancel, price acceptance or real RePrice was called.

For supplier fare 5,263.48 with the user's active 1% markup and gross 5,749.00:

| Live account | Share | Discount | Agent payable |
| --- | ---: | ---: | ---: |
| Basic | 55% | 238.09 | 5,510.91 |
| Enterprise | 100% | 432.89 | 5,316.11 |

Professional 85% is verified by regression: discount 367.96, payable 5,381.04. A separate supplier fare naturally gives different discount/payable values, verified against its immutable original. No private supplier amounts are exposed to B2B responses.

Saved shares 55/85/100 and the active markup rule remain unchanged. Booking count stayed 5 and issue count 0. No working schema migration or production deployment occurred. Local read-only verification evidence is `.local/evidence/b2b-gross-20260917/live-tier-discount.json`.

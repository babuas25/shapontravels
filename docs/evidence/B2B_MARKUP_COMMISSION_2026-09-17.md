# B2B markup commission clarification — 17 September 2026

Historical: superseded by the later [tier discount clarification](B2B_TIER_DISCOUNT_2026-09-17.md).

Final user confirmation: agent payable is published gross minus commission. Commission is the configured tier share of the resolved supplier-based markup, not the gross-to-supplier difference and not a deduction from supplier plus markup.

For one adult, supplier 5,263.48, base 4,624, taxes 1,125, markup 5%: gross 5,749.00, effective markup 263.17. Shares 60/90/100% yield commission 157.90/236.85/263.17 and Agent payable 5,591.10/5,512.15/5,485.83. Rounded supplier-plus-markup is 5,526.65, an intermediate amount only. Gross stays unchanged across tiers.

Search and RePrice pass the resolved winning markup to the same snapshot calculation. The original supplier total is the percentage basis; the existing exact decimal engine rounds supplier-plus-markup before deriving the effective markup. Commission rounds per passenger before counts. Negative resulting payable is rejected. Snapshots and wallet accepted-payable reconciliation still use gross = commission + payable. Old accepted snapshots are not rewritten.

B2B cards show Gross above Agent. Tier names/share percentages are removed from the card; commission is available in the expanded fare information. Staff gross/supplier display is preserved. REQUIREMENTS, B2B_TIERS, MARKUP_API and portal documentation now describe the same final rule.

Validation: 70 Rust library tests passed (one private recovery test ignored); six foundation and six production-fixture tests passed. Full disposable PostgreSQL integration passed in 46.59 seconds, including explicit fixed-markup commission checks in Search and RePrice, tier/policy changes, acceptance, and simulated booking snapshot stability. Strict Clippy passed. Frontend hermetic prebooking regressions, TypeScript, targeted ESLint and whitespace checks passed.

Local backend was rebuilt/restarted and readiness returned 200. Live search through the existing canonical B2B identity path for DAC–JSR, 30 September 2026, returned three retained offers. Enterprise 100% returned gross 5,749.00, commission 263.17 and payable 5,485.83 for the target supplier fare. Another supplier fare returned commission 269.75 and payable 5,479.25, as determined by its own markup basis. No private supplier amount was exposed to B2B.

Saved local shares were discovered to be 55/85/100, rather than the user's hypothetical 60/90/100. The user explicitly chose to retain 55/85/100; no policy was changed. At the target supplier fare, Basic 55% commission/payable are 144.74/5,604.26; Professional 85% are 223.69/5,525.31; Enterprise 100% are 263.17/5,485.83. Search/session/pricing endpoints only were used against live suppliers. Booking count remained 5 and issue count 0. No real booking, issue, cancellation or acceptance, production deployment or working schema migration was performed. Local evidence: `.local/evidence/b2b-gross-20260917/live-markup-search.json`.
